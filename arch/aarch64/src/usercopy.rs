//! Cópia entre o kernel e a memória de usuário: a única rotina que toca endereço de usuário.
//! Usa `ldtr`/`sttr` (acesso não privilegiado) com o PAN ligado o tempo todo. A falta durante a
//! cópia não para o kernel: o `elr` cai no ponto de recuperação e a rotina devolve os bytes que
//! faltaram (SDD 17, "Cópia usuário↔kernel").

use core::arch::global_asm;
use core::ptr;

// `rustos_copy_from_user(dst, src, len)` e `rustos_copy_to_user(dst, src, len)`: devolvem em `x0`
// quantos bytes ficaram sem copiar. A falta em `ldtrb`/`sttrb` leva o `elr` a
// `rustos_usercopy_done` com `x2` = o que faltou.
global_asm!(
    r#"
    .section .text.rustos_usercopy, "ax"
    .balign 16
    .global rustos_copy_from_user
rustos_copy_from_user:
    cbz x2, rustos_usercopy_done
1:
    .global rustos_usercopy_load
rustos_usercopy_load:
    ldtrb w3, [x1]
    strb w3, [x0], #1
    add x1, x1, #1
    subs x2, x2, #1
    b.ne 1b
    .global rustos_usercopy_done
rustos_usercopy_done:
    mov x0, x2
    ret

    .balign 16
    .global rustos_copy_to_user
rustos_copy_to_user:
    cbz x2, rustos_usercopy_done
2:
    ldrb w3, [x1], #1
    .global rustos_usercopy_store
rustos_usercopy_store:
    sttrb w3, [x0]
    add x0, x0, #1
    subs x2, x2, #1
    b.ne 2b
    b rustos_usercopy_done
    "#
);

unsafe extern "C" {
    fn rustos_copy_from_user(dst: *mut u8, src: *const u8, len: usize) -> usize;
    fn rustos_copy_to_user(dst: *mut u8, src: *const u8, len: usize) -> usize;
    static rustos_usercopy_load: u8;
    static rustos_usercopy_store: u8;
    static rustos_usercopy_done: u8;
}

/// Copia `len` bytes de `src` (usuário) para `dst` (kernel). Devolve os bytes que faltaram.
///
/// # Safety
/// `dst` é memória do kernel gravável por `len` bytes; `[src, src + len)` está em
/// `[USER_MIN, USER_TOP)` (um `UserRange`), de modo que a cópia só toca a metade de usuário.
pub unsafe fn copy_from_user(dst: *mut u8, src: u64, len: usize) -> usize {
    let src = ptr::with_exposed_provenance::<u8>(usize::try_from(src).unwrap_or(usize::MAX));
    // SAFETY: o contrato da função; uma falta em `src` cai no ponto de recuperação.
    unsafe { rustos_copy_from_user(dst, src, len) }
}

/// Copia `len` bytes de `src` (kernel) para `dst` (usuário). Devolve os bytes que faltaram.
///
/// # Safety
/// `src` é memória do kernel legível por `len` bytes; `[dst, dst + len)` está em
/// `[USER_MIN, USER_TOP)` (um `UserRange`).
pub unsafe fn copy_to_user(dst: u64, src: *const u8, len: usize) -> usize {
    let dst = ptr::with_exposed_provenance_mut::<u8>(usize::try_from(dst).unwrap_or(usize::MAX));
    // SAFETY: o contrato da função; uma falta em `dst` cai no ponto de recuperação.
    unsafe { rustos_copy_to_user(dst, src, len) }
}

/// O `elr` da falta está numa das instruções de cópia: trata-se aqui, sem matar nada.
pub fn is_usercopy_fault(elr: u64) -> bool {
    let load = ptr::addr_of!(rustos_usercopy_load).addr() as u64;
    let store = ptr::addr_of!(rustos_usercopy_store).addr() as u64;
    elr == load || elr == store
}

/// Para onde o `elr` vai depois da falta: o fim da cópia, com `x2` = o que faltou.
pub fn recovery_address() -> u64 {
    ptr::addr_of!(rustos_usercopy_done).addr() as u64
}
