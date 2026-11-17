//! Troca de contexto entre threads do kernel: os registradores callee-saved
//! ficam na pilha de quem sai e o `rsp` é a única coisa guardada em `Context`.

use core::arch::global_asm;
use core::ptr;

use rustos_hal::VirtAddr;

/// O ponto de retomada de uma thread parada: o `rsp` com os seis registradores
/// callee-saved e o endereço de retorno empilhados em cima dele.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct Context {
    sp: u64,
}

/// Quatro registradores levam a thread nova até a função dela; ver `Context::new`.
const FRAME_WORDS: usize = 7;
/// O `rsp` na primeira instrução do trampolim precisa ser múltiplo de 16.
const TOP_RESERVE: u64 = 16;

// `switch`: empilha rbp, rbx, r12–r15, guarda o `rsp` em `from`, carrega o de
// `to` e desempilha na ordem inversa. `task_start` é o primeiro `ret` de uma
// thread nova: arg em r12, entrada em rbx, retorno em r13.
global_asm!(
    r#"
    .section .text.rustos_switch, "ax"
    .balign 16
    .global rustos_context_switch
rustos_context_switch:
    push rbp
    push rbx
    push r12
    push r13
    push r14
    push r15
    mov [rdi], rsp
    mov rsp, [rsi]
    pop r15
    pop r14
    pop r13
    pop r12
    pop rbx
    pop rbp
    ret

    .balign 16
    .global rustos_task_start
rustos_task_start:
    mov rdi, r12
    call rbx
    call r13
    ud2
    "#
);

unsafe extern "C" {
    fn rustos_context_switch(from: *mut Context, to: *const Context);
    static rustos_task_start: u8;
}

impl Context {
    /// Contexto vazio: só serve de destino de `switch` para guardar quem sai.
    pub const fn empty() -> Self {
        Self { sp: 0 }
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
        let sp = top.as_u64() - TOP_RESERVE - (FRAME_WORDS as u64 * 8);
        let base = usize::try_from(sp).unwrap_or(0);
        let start = ptr::addr_of!(rustos_task_start).addr() as u64;
        // r15, r14, r13, r12, rbx, rbp e o endereço de retorno, do menor endereço ao maior.
        let frame: [u64; FRAME_WORDS] = [
            0,
            0,
            on_return as usize as u64,
            arg as u64,
            entry as usize as u64,
            0,
            start,
        ];
        let frame_at = ptr::with_exposed_provenance_mut::<[u64; FRAME_WORDS]>(base);
        // SAFETY: `[sp, sp + 56)` está dentro da pilha que o chamador entregou.
        unsafe { frame_at.write(frame) };
        Self { sp }
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
