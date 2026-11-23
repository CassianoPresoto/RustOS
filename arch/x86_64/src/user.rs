//! A entrada e a saída do anel 3: os MSRs de `SYSCALL` em cada CPU, a rotina de entrada, a
//! primeira volta ao usuário e a troca do espaço de endereçamento desta CPU (SDD 17).

use core::arch::{asm, global_asm};
use core::ptr;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::percpu::prefix;
use crate::prefix::{CpuPrefix, OFFSET_KERNEL_RSP, OFFSET_USER_RSP};
use crate::segments::{USER_CODE, USER_DATA};
use crate::sysreg::{MSR_EFER, MSR_FMASK, MSR_LSTAR, MSR_STAR, rdmsr, wrmsr};
use crate::sysret::{FMASK, STAR, USER_RFLAGS};
use crate::traps::SYSCALL_VECTOR;

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

/// Põe no `CR3` a raiz da tarefa (`0`: a do kernel) e, se é de usuário, `kernel_stack_top` no `RSP0`
/// e em `gs:[kernel_rsp]`. Só escreve se a raiz mudou; o alocador garante que nenhuma raiz de processo é `0`.
/// # Safety
/// Ring 0, mascarada; `root` mapeia o kernel inteiro na metade alta.
pub unsafe fn switch_space(root: u64, kernel_stack_top: u64) {
    let cpu = prefix();
    let target = if root == 0 {
        KERNEL_ROOT.load(Ordering::Acquire)
    } else {
        root
    };
    if cpu.active_root.load(Ordering::Relaxed) != target {
        // SAFETY: o contrato da função; `target` mapeia o código e a pilha em uso.
        unsafe { asm!("mov cr3, {}", in(reg) target, options(nostack, preserves_flags)) };
        cpu.active_root.store(target, Ordering::Relaxed);
    }
    if root != 0 {
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
