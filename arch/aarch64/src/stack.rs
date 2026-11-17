//! Troca de pilha: o Limine entrega a pilha em memória que ele mesmo reaproveita.

use core::arch::asm;

use rustos_hal::VirtAddr;

/// Função que roda na pilha nova e nunca volta; recebe `arg` em `x0`.
pub type Continuation = unsafe extern "C" fn(*mut ()) -> !;

/// Passa a rodar em `top` e chama `entry(arg)`.
/// # Safety
/// `top` é o fim, alinhado a 16, de uma pilha mapeada só desta CPU. Nada da pilha
/// antiga é lido depois, e `entry` nunca retorna.
pub unsafe fn call_on_stack(top: VirtAddr, entry: Continuation, arg: *mut ()) -> ! {
    // SAFETY: o contrato da função dá a pilha nova e a continuação que não volta;
    // `brk` só existe para o caso de `entry` violar esse contrato.
    unsafe {
        asm!(
            "mov sp, {top}",
            "mov x29, xzr",
            "blr {entry}",
            "brk #1",
            top = in(reg) top.as_u64(),
            entry = in(reg) entry,
            in("x0") arg,
            options(noreturn),
        );
    }
}
