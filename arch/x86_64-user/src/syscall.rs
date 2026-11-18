//! O stub de `syscall`: número em `rax`, argumentos em `rdi`, `rsi`, `rdx`, `r10`, `r8` e `r9`;
//! volta o status em `rax` e o valor em `rdx`. O hardware sobrescreve `rcx` e `r11`.

use core::arch::asm;

use rustos_hal::abi::{SyscallArgs, SyscallReturn};

/// Faz a chamada de sistema `call`.
///
/// # Safety
/// Todo ponteiro em `call.args` é de memória do chamador, viva e do tamanho que a chamada diz.
pub unsafe fn syscall(call: &SyscallArgs) -> SyscallReturn {
    let [a0, a1, a2, a3, a4, a5] = call.args;
    let (status, value);
    // SAFETY: o contrato da função; `syscall` só mexe em `rax`, `rdx`, `rcx` e `r11`, declarados.
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") call.number => status,
            in("rdi") a0,
            in("rsi") a1,
            inlateout("rdx") a2 => value,
            in("r10") a3,
            in("r8") a4,
            in("r9") a5,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    SyscallReturn { status, value }
}
