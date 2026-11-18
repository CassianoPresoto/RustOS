//! Cópia entre o kernel e a memória de usuário: a única rotina que toca endereço de usuário,
//! sob SMAP (`stac`/`clac` em volta). A falta durante a cópia não para o kernel: o `pc` cai no
//! ponto de recuperação e a rotina devolve os bytes que faltaram (SDD 17, "Cópia usuário↔kernel").

use core::arch::global_asm;
use core::ptr;
use core::sync::atomic::{AtomicBool, Ordering};

/// A CPU tem SMAP: sem ele `stac` e `clac` são `#UD`.
pub(crate) static SMAP_ON: AtomicBool = AtomicBool::new(false);

/// Diz à rotina se `stac` e `clac` existem. Uma vez, no boot, antes da primeira cópia.
pub fn set_smap(on: bool) {
    SMAP_ON.store(on, Ordering::Relaxed);
}

// `rustos_copy_user(dst, src, len)`: devolve em `rax` quantos bytes ficaram sem copiar. Uma falta
// em `rep movsb` leva o `rip` a `rustos_usercopy_done` com `rcx` = o que faltou.
global_asm!(
    r#"
    .section .text.rustos_usercopy, "ax"
    .balign 16
    .global rustos_copy_user
rustos_copy_user:
    mov rcx, rdx
    cmp byte ptr [rip + {smap}], 0
    je 2f
    stac
2:
    .global rustos_usercopy_insn
rustos_usercopy_insn:
    rep movsb
    .global rustos_usercopy_done
rustos_usercopy_done:
    cmp byte ptr [rip + {smap}], 0
    je 3f
    clac
3:
    mov rax, rcx
    ret
    "#,
    smap = sym SMAP_ON,
);

unsafe extern "C" {
    fn rustos_copy_user(dst: *mut u8, src: *const u8, len: usize) -> usize;
    static rustos_usercopy_insn: u8;
    static rustos_usercopy_done: u8;
}

/// Copia `len` bytes de `src` (usuário) para `dst` (kernel). Devolve os bytes que faltaram.
///
/// # Safety
/// `dst` é memória do kernel gravável por `len` bytes; `[src, src + len)` é um `UserRange`.
pub unsafe fn copy_from_user(dst: *mut u8, src: u64, len: usize) -> usize {
    let src = ptr::with_exposed_provenance::<u8>(usize::try_from(src).unwrap_or(usize::MAX));
    // SAFETY: o contrato da função; uma falta em `src` cai no ponto de recuperação.
    unsafe { rustos_copy_user(dst, src, len) }
}

/// Copia `len` bytes de `src` (kernel) para `dst` (usuário). Devolve os bytes que faltaram.
///
/// # Safety
/// `src` é memória do kernel legível por `len` bytes; `[dst, dst + len)` é um `UserRange`.
pub unsafe fn copy_to_user(dst: u64, src: *const u8, len: usize) -> usize {
    let dst = ptr::with_exposed_provenance_mut::<u8>(usize::try_from(dst).unwrap_or(usize::MAX));
    // SAFETY: o contrato da função; uma falta em `dst` cai no ponto de recuperação.
    unsafe { rustos_copy_user(dst, src, len) }
}

/// O `rip` da falta está na instrução de cópia: a falta se trata aqui, sem matar nada.
pub fn is_usercopy_fault(rip: u64) -> bool {
    rip == ptr::addr_of!(rustos_usercopy_insn).addr() as u64
}

/// Para onde o `rip` vai depois da falta: o fim da cópia, com `rcx` = o que faltou.
pub fn recovery_address() -> u64 {
    ptr::addr_of!(rustos_usercopy_done).addr() as u64
}
