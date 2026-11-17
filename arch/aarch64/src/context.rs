//! Troca de contexto entre threads do kernel: x19–x30 e o `sp` de cada thread
//! ficam em `Context`. O kernel roda em EL1h, com uma pilha por thread em `SP_EL1`.

use core::arch::global_asm;
use core::ptr;

use rustos_hal::VirtAddr;

/// Registradores callee-saved (x19–x30) e `sp` de uma thread parada.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct Context {
    x: [u64; 12],
    sp: u64,
}

const X_ENTRY: usize = 0;
const X_ARG: usize = 1;
const X_ON_RETURN: usize = 2;
const X_FRAME: usize = 10;
const X_LINK: usize = 11;
const _: () = assert!(size_of::<Context>() == 104);

// `switch`: x0 = `from`, x1 = `to`. `task_start` é o primeiro `ret` de uma thread
// nova: entrada em x19, arg em x20, retorno em x21.
global_asm!(
    r#"
    .section .text.rustos_switch, "ax"
    .balign 16
    .global rustos_context_switch
rustos_context_switch:
    mov x9, sp
    stp x19, x20, [x0]
    stp x21, x22, [x0, #16]
    stp x23, x24, [x0, #32]
    stp x25, x26, [x0, #48]
    stp x27, x28, [x0, #64]
    stp x29, x30, [x0, #80]
    str x9, [x0, #96]
    ldp x19, x20, [x1]
    ldp x21, x22, [x1, #16]
    ldp x23, x24, [x1, #32]
    ldp x25, x26, [x1, #48]
    ldp x27, x28, [x1, #64]
    ldp x29, x30, [x1, #80]
    ldr x9, [x1, #96]
    mov sp, x9
    ret

    .balign 16
    .global rustos_task_start
rustos_task_start:
    mov x0, x20
    blr x19
    blr x21
    brk #1
    "#
);

unsafe extern "C" {
    fn rustos_context_switch(from: *mut Context, to: *const Context);
    static rustos_task_start: u8;
}

impl Context {
    /// Contexto vazio: só serve de destino de `switch` para guardar quem sai.
    pub const fn empty() -> Self {
        Self { x: [0; 12], sp: 0 }
    }

    /// Thread nova: na primeira troca executa `entry(arg)` e depois `on_return()`.
    ///
    /// # Safety
    /// `top` é o fim, alinhado a 16, de uma pilha mapeada só desta thread.
    pub unsafe fn new(
        top: VirtAddr,
        entry: extern "C" fn(usize),
        arg: usize,
        on_return: extern "C" fn() -> !,
    ) -> Self {
        let mut context = Self::empty();
        context.x[X_ENTRY] = entry as usize as u64;
        context.x[X_ARG] = arg as u64;
        context.x[X_ON_RETURN] = on_return as usize as u64;
        context.x[X_FRAME] = 0;
        context.x[X_LINK] = ptr::addr_of!(rustos_task_start).addr() as u64;
        context.sp = top.as_u64();
        context
    }
}

/// Guarda o contexto atual em `from` e executa o de `to`.
///
/// # Safety
/// `to` vem de `Context::new` ou de uma troca; IRQ mascarada e nenhum lock preso.
pub unsafe fn switch(from: *mut Context, to: *const Context) {
    // SAFETY: o contrato da função; o `asm` só mexe em registradores e na pilha das duas threads.
    unsafe { rustos_context_switch(from, to) };
}
