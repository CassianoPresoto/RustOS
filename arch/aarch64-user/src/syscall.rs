//! O stub de `svc`: número em `x8`, argumentos em `x0` a `x5`; volta o status em `x0` e o valor em
//! `x1`. Os outros registradores ficam como estavam.

use core::arch::asm;

use rustos_hal::abi::{SyscallArgs, SyscallReturn};

/// Faz a chamada de sistema `call`.
///
/// # Safety
/// Todo ponteiro em `call.args` é de memória do chamador, viva e do tamanho que a chamada diz.
pub unsafe fn syscall(call: &SyscallArgs) -> SyscallReturn {
    let [a0, a1, a2, a3, a4, a5] = call.args;
    let (status, value);
    // SAFETY: o contrato da função; `svc #0` só devolve em `x0` e `x1`, declarados.
    unsafe {
        asm!(
            "svc #0",
            inlateout("x0") a0 => status,
            inlateout("x1") a1 => value,
            in("x2") a2,
            in("x3") a3,
            in("x4") a4,
            in("x5") a5,
            in("x8") call.number,
            options(nostack),
        );
    }
    SyscallReturn { status, value }
}
