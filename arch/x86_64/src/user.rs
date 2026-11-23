//! A entrada e a saída do anel 3: os MSRs de `SYSCALL` em cada CPU, a rotina de entrada, a
//! primeira volta ao usuário e a troca do espaço de endereçamento desta CPU (SDD 17).

use core::arch::{asm, global_asm};
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::percpu::prefix;
use crate::prefix::{CpuPrefix, OFFSET_KERNEL_RSP, OFFSET_USER_RSP};
use crate::segments::{USER_CODE, USER_DATA};
use crate::sysreg::{
    CR3_NOFLUSH, CR3_PCID_MASK, CR4_PCIDE, CR4_PGE, MSR_EFER, MSR_FMASK, MSR_LSTAR, MSR_STAR,
    rdmsr, read_cr4, write_cr4, wrmsr,
};
use crate::sysret::{FMASK, STAR, USER_RFLAGS};
use crate::traps::SYSCALL_VECTOR;
use rustos_hal::{TagFlush, TlbTag};

/// `EFER.SCE`: liga `SYSCALL` e `SYSRET`.
const EFER_SCE: u64 = 1;

// `SYSCALL` não troca a pilha: guarda o `rsp` do usuário, carrega o do kernel e monta o mesmo
// quadro que uma exceção, para sair pelo mesmo caminho (`rustos_isr_common_swapped`).
global_asm!(
    r#"
    .section .text.rustos_syscall, "ax"
    .balign 16
    .global rustos_syscall_entry
rustos_syscall_entry:
    swapgs
    mov gs:[{user_rsp}], rsp
    mov rsp, gs:[{kernel_rsp}]
    push {user_ss}
    push qword ptr gs:[{user_rsp}]
    push r11
    push {user_cs}
    push rcx
    push 0
    push {vector}
    jmp rustos_isr_common_swapped
    "#,
    user_rsp = const OFFSET_USER_RSP,
    kernel_rsp = const OFFSET_KERNEL_RSP,
    user_ss = const USER_DATA,
    user_cs = const USER_CODE,
    vector = const SYSCALL_VECTOR,
);

unsafe extern "C" {
    static rustos_syscall_entry: u8;
}

/// Liga `SYSCALL` nesta CPU: `EFER.SCE`, `STAR`, `LSTAR` e `FMASK`. Cada núcleo faz o seu.
///
/// # Safety
/// Ring 0, depois de o bloco por CPU e a IDT existirem: a primeira `SYSCALL` já usa o `GS`.
pub unsafe fn init_syscall() {
    // SAFETY: ring 0; `EFER` existe em todo x86_64.
    let efer = unsafe { rdmsr(MSR_EFER) };
    // SAFETY: só liga `SCE`; o x86_64 de 64 bits sempre tem `SYSCALL`.
    unsafe { wrmsr(MSR_EFER, efer | EFER_SCE) };
    // SAFETY: os selectors de `STAR` são os da GDT de `segments`.
    unsafe { wrmsr(MSR_STAR, STAR) };
    // SAFETY: `LSTAR` aponta para a entrada acima, em `.text`.
    unsafe { wrmsr(MSR_LSTAR, ptr::addr_of!(rustos_syscall_entry).addr() as u64) };
    // SAFETY: `FMASK` é só uma máscara de bits de `RFLAGS`.
    unsafe { wrmsr(MSR_FMASK, FMASK) };
}

/// `EFLAGS.AC` está ligado agora: o kernel rodando assim tem o SMAP desarmado.
pub fn ac_flag_set() -> bool {
    let flags: u64;
    // SAFETY: `pushfq` e `pop` só leem o `RFLAGS`; a pilha tem espaço para uma palavra.
    unsafe { asm!("pushfq", "pop {}", out(reg) flags, options(preserves_flags)) };
    flags & AC_BIT != 0
}

/// `RFLAGS.AC`, bit 18.
const AC_BIT: u64 = 1 << 18;

/// A raiz do kernel: o que o `CR3` recebe quando a tarefa não é de usuário.
static KERNEL_ROOT: AtomicU64 = AtomicU64::new(0);

/// Guarda a raiz do kernel (depois de `Vm::activate`). Uma vez, antes de qualquer troca de espaço.
pub fn set_kernel_root(root: u64) {
    KERNEL_ROOT.store(root, Ordering::Release);
}

/// As etiquetas de TLB (PCID) estão ligadas em todas as CPUs: decidido pelo BSP em `enable_tags`.
static TAGGED: AtomicBool = AtomicBool::new(false);
/// `INVPCID` existe nesta máquina (`CPUID.7.0:EBX[10]`); sem ele, esvaziar tudo é `CR4.PGE` e a
/// troca de uma etiqueta esvazia tudo.
static INVPCID: AtomicBool = AtomicBool::new(false);

/// Liga `CR4.PCIDE` no BSP, se o `CR3` do kernel tem os 12 bits baixos zerados (o `CR4` não aceita
/// a troca com eles acesos). Devolve se as etiquetas valem para todas as CPUs.
/// # Safety
/// Ring 0, com a raiz do kernel no `CR3`, antes de qualquer espaço de usuário.
pub unsafe fn enable_tags(invpcid: bool) -> bool {
    let cr3: u64;
    // SAFETY: ler `CR3` não tem efeito; ring 0.
    unsafe { asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack, preserves_flags)) };
    if cr3 & CR3_PCID_MASK != 0 {
        return false;
    }
    INVPCID.store(invpcid, Ordering::Relaxed);
    TAGGED.store(true, Ordering::Release);
    // SAFETY: ring 0; `PCIDE` só muda a interpretação de `CR3[11:0]`, que é zero.
    unsafe { write_cr4(read_cr4() | CR4_PCIDE) };
    true
}

/// Põe `CR4.PCIDE` nesta CPU, se as etiquetas valem (cada núcleo tem o seu `CR4`). Devolve se a CPU
/// ficou com as etiquetas que o BSP ligou: falso se o `CR3` dela tem bits de PCID acesos.
/// # Safety
/// Ring 0, com a raiz do kernel no `CR3`, depois de `enable_tags` no BSP.
pub unsafe fn load_tags_cpu() -> bool {
    if !TAGGED.load(Ordering::Acquire) {
        return true;
    }
    let cr3: u64;
    // SAFETY: ler `CR3` não tem efeito; ring 0.
    unsafe { asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack, preserves_flags)) };
    if cr3 & CR3_PCID_MASK != 0 {
        return false;
    }
    // SAFETY: o mesmo que `enable_tags`: `CR3[11:0]` é zero.
    unsafe { write_cr4(read_cr4() | CR4_PCIDE) };
    true
}

/// `INVPCID` com o descritor de 16 bytes: o PCID (com os bits altos zero) e o endereço linear.
/// # Safety
/// Ring 0 com `CR4.PCIDE` ligado (ou só tipos 2 e 3, sem ele); `kind` é 0 a 3.
unsafe fn invpcid(kind: u64, pcid: u64, address: u64) {
    let descriptor = [pcid, address];
    // SAFETY: o contrato da função; o descritor vive na pilha durante a instrução.
    unsafe {
        asm!(
            "invpcid {kind}, [{desc}]",
            kind = in(reg) kind,
            desc = in(reg) descriptor.as_ptr(),
            options(nostack, readonly, preserves_flags),
        );
    }
}

/// Esvazia o TLB inteiro desta CPU, inclusive o global: `INVPCID` tipo 2 ou, sem ele, o `CR4.PGE`
/// desligado e de volta (um `mov cr3` sem `NOFLUSH` só esvazia o PCID carregado).
fn flush_all_local() {
    if INVPCID.load(Ordering::Relaxed) {
        // SAFETY: ring 0; tipo 2 vale com ou sem PCIDE, e o descritor é zero.
        unsafe { invpcid(2, 0, 0) };
    } else {
        let cr4 = read_cr4();
        // SAFETY: ring 0; mudar `PGE` esvazia o TLB e o valor volta ao que era.
        unsafe { write_cr4(cr4 ^ CR4_PGE) };
        // SAFETY: o mesmo contrato; restaura o valor de antes.
        unsafe { write_cr4(cr4) };
    }
}

/// Põe no `CR3` a raiz da tarefa (`0`: a do kernel, PCID 0) e, se é de usuário, `kernel_stack_top`
/// no `RSP0`. Esvazia o que `flush` pede antes de carregar; sem etiqueta, escreve se a raiz mudou.
/// # Safety
/// Ring 0, mascarada; `root` mapeia o kernel na metade alta; `Tagged` só com `enable_tags` ligado.
pub unsafe fn switch_space(root: u64, tag: TlbTag, kernel_stack_top: u64) {
    let cpu = prefix();
    let kernel = root == 0;
    let target = if kernel {
        KERNEL_ROOT.load(Ordering::Acquire)
    } else {
        root
    };
    match tag {
        TlbTag::Untagged => {
            if cpu.active_root.load(Ordering::Relaxed) != target {
                // SAFETY: o contrato da função; `target` mapeia o código e a pilha em uso.
                unsafe { asm!("mov cr3, {}", in(reg) target, options(nostack, preserves_flags)) };
                cpu.active_root.store(target, Ordering::Relaxed);
            }
        }
        TlbTag::Tagged { index, flush } => {
            let pcid = if kernel { 0 } else { u64::from(index) };
            let value = target | pcid;
            match flush {
                TagFlush::Keep => {}
                TagFlush::Space => {
                    if INVPCID.load(Ordering::Relaxed) {
                        // SAFETY: ring 0; tipo 1 invalida só este PCID, sem endereço.
                        unsafe { invpcid(1, pcid, 0) };
                    } else {
                        flush_all_local();
                    }
                }
                TagFlush::All => flush_all_local(),
            }
            if flush != TagFlush::Keep || cpu.active_root.load(Ordering::Relaxed) != value {
                // SAFETY: o contrato da função; `value` tem o PCID da raiz e `NOFLUSH` já cuidou do TLB.
                unsafe {
                    asm!("mov cr3, {}", in(reg) value | CR3_NOFLUSH, options(nostack, preserves_flags));
                }
                cpu.active_root.store(value, Ordering::Relaxed);
            }
        }
    }
    if !kernel {
        set_entry_stack(cpu, kernel_stack_top);
    }
}

fn set_entry_stack(cpu: &CpuPrefix, top: u64) {
    cpu.kernel_rsp.store(top, Ordering::Relaxed);
    let rsp0 = ptr::with_exposed_provenance_mut::<u64>(
        usize::try_from(cpu.tss_rsp0.load(Ordering::Relaxed)).unwrap_or(0),
    );
    // SAFETY: `tss_rsp0` é o endereço do `rsp[0]` da TSS desta CPU, posto em `load_cpu`; o
    // campo da TSS é `packed(4)`, então a escrita não é alinhada.
    unsafe { rsp0.write_unaligned(top) };
}

/// Primeira volta ao anel 3: quadro de `iretq` no topo da pilha, registradores zero (`rdi` e `rsi`
/// levam `arg0` e `arg1`), `swapgs`. Nunca volta.
/// # Safety
/// Ring 0, mascarada, depois de `switch_space`; `entry` e `sp` são de usuário.
pub unsafe fn enter_user(entry: u64, sp: u64, arg0: u64, arg1: u64) -> ! {
    // SAFETY: o contrato da função; o quadro é o que a CPU espera de um `iretq` para o anel 3.
    unsafe {
        asm!(
            "mov rsp, gs:[{kernel_rsp}]",
            "push {user_ss}",
            "push {sp}",
            "push {rflags}",
            "push {user_cs}",
            "push {entry}",
            "mov ax, {user_ss}",
            "mov ds, ax",
            "mov es, ax",
            "xor eax, eax",
            "xor ebx, ebx",
            "xor ecx, ecx",
            "xor edx, edx",
            "xor ebp, ebp",
            "xor r8d, r8d",
            "xor r9d, r9d",
            "xor r10d, r10d",
            "xor r11d, r11d",
            "xor r12d, r12d",
            "xor r13d, r13d",
            "xor r14d, r14d",
            "xor r15d, r15d",
            "swapgs",
            "iretq",
            kernel_rsp = const OFFSET_KERNEL_RSP,
            user_ss = const USER_DATA,
            user_cs = const USER_CODE,
            rflags = const USER_RFLAGS,
            sp = in(reg) sp,
            entry = in(reg) entry,
            in("rdi") arg0,
            in("rsi") arg1,
            options(noreturn),
        );
    }
}
