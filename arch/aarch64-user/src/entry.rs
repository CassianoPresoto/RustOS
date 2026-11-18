//! O `_start`: o kernel entra em EL0 com `sp` apontando `argc`, alinhado a 16, `x0` com o handle de
//! boot e o resto zero. Chama `rustos_user_main(pilha_inicial, handle)` do `user/rt`.

use core::arch::global_asm;

global_asm!(
    r#"
    .section .text._start, "ax"
    .global _start
    .type _start, %function
_start:
    mov x29, xzr
    mov x30, xzr
    mov x1, x0
    mov x0, sp
    and x2, x0, #-16
    mov sp, x2
    bl rustos_user_main
    brk #0
    .size _start, . - _start
    "#
);
