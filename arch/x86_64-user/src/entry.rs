//! O `_start`: o kernel entra com `rsp` apontando `argc`, alinhado a 16, `rdi` com o handle de
//! boot e o resto zero. Chama `rustos_user_main(pilha_inicial, handle)` do `user/rt`.

use core::arch::global_asm;

global_asm!(
    r#"
    .section .text._start, "ax"
    .global _start
    .type _start, @function
_start:
    xor ebp, ebp
    mov rsi, rdi
    mov rdi, rsp
    and rsp, -16
    call rustos_user_main
    ud2
    .size _start, . - _start
    "#
);
