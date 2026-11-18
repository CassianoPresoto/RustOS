//! O programa de usuário de teste da fatia 4a, embutido no kernel só para o modo de teste
//! (feature `selftest`): o kernel copia `[start, end)` para uma página de usuário. Código
//! independente de posição; cada entrada é um caso do autoteste (SDD 17).

use core::arch::global_asm;
use core::ptr;

use rustos_hal::abi::Syscall;

const HELLO: &str = "hello from el0\n";

global_asm!(
    r#"
    .section .text.rustos_user_blob, "ax"
    .balign 16
    .global rustos_user_blob_start
rustos_user_blob_start:
    .global rustos_user_hello
    .global rustos_user_fault_mem
    .global rustos_user_fault_priv
    .global rustos_user_fault_trap
    .global rustos_user_fault_stack
    .global rustos_user_flags
    .global rustos_user_null_loop
    .global rustos_user_blob_end

.Lhello_msg:
    .ascii "hello from el0\n"
    .balign 4

rustos_user_hello:
    adr x0, .Lhello_msg
    mov x1, #{hello_len}
    mov x8, #{debug_write}
    svc #0
    cbnz x0, .Lfail1
    mov x8, #{clock}
    svc #0
    cbnz x0, .Lfail2
    cbz x1, .Lfail2
    sub sp, sp, #64
    mov x0, sp
    mov x1, #32
    mov x8, #{random}
    svc #0
    add sp, sp, #64
    cbnz x0, .Lfail3
    mov x8, #0x7777
    svc #0
    cmn x0, #7
    b.ne .Lfail4
    mov x8, #{thread_exit}
    svc #0
    cmn x0, #7
    b.ne .Lfail4
    mov x0, #0x20000
    mov x1, #8
    mov x8, #{debug_write}
    svc #0
    cmn x0, #8
    b.ne .Lfail5
    mov x0, #0
    b .Lexit
.Lfail1:
    mov x0, #1
    b .Lexit
.Lfail2:
    mov x0, #2
    b .Lexit
.Lfail3:
    mov x0, #3
    b .Lexit
.Lfail4:
    mov x0, #4
    b .Lexit
.Lfail5:
    mov x0, #5
.Lexit:
    mov x8, #{process_exit}
    svc #0
    brk #0

    .balign 16
rustos_user_fault_mem:
    ldr x1, [x0]
    mov x0, #0
    mov x8, #{process_exit}
    svc #0
    brk #0

    .balign 16
rustos_user_fault_priv:
    msr ttbr0_el1, xzr
    mov x0, #0
    mov x8, #{process_exit}
    svc #0
    brk #0

    .balign 16
rustos_user_fault_trap:
    brk #0
    mov x0, #0
    mov x8, #{process_exit}
    svc #0
    brk #0

    .balign 16
rustos_user_fault_stack:
    mov x1, #1
    mov sp, x1
    ldr x0, [sp]
    mov x0, #0
    mov x8, #{process_exit}
    svc #0
    brk #0

    .balign 16
rustos_user_flags:
    mov x9, x0
    mrs x10, cntvct_el0
1:
    mov x8, #0
    svc #0
    mrs x11, cntvct_el0
    sub x11, x11, x10
    cmp x11, x9
    b.lo 1b
    mov x0, #0
    mov x8, #{process_exit}
    svc #0
    brk #0

    .balign 16
rustos_user_null_loop:
    mov x9, x0
    mrs x10, cntvct_el0
1:
    mov x8, #0
    svc #0
    subs x9, x9, #1
    b.ne 1b
    mrs x11, cntvct_el0
    sub x0, x11, x10
    mov x8, #{process_exit}
    svc #0
    brk #0
rustos_user_blob_end:
    "#,
    hello_len = const HELLO.len(),
    debug_write = const Syscall::DebugWrite as u64,
    process_exit = const Syscall::ProcessExit as u64,
    thread_exit = const Syscall::ThreadExit as u64,
    clock = const Syscall::ClockGetMonotonic as u64,
    random = const Syscall::RandomGet as u64,
);

unsafe extern "C" {
    static rustos_user_blob_start: u8;
    static rustos_user_blob_end: u8;
    static rustos_user_hello: u8;
    static rustos_user_fault_mem: u8;
    static rustos_user_fault_priv: u8;
    static rustos_user_fault_trap: u8;
    static rustos_user_fault_stack: u8;
    static rustos_user_flags: u8;
    static rustos_user_null_loop: u8;
}

/// Os casos do programa.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Case {
    /// Escreve uma linha por `debug_write`, chama as outras chamadas e sai com 0 (ou com o
    /// número do passo que falhou).
    Hello,
    /// Lê o endereço que recebe em `x0`.
    FaultMemory,
    /// Escreve num registrador de sistema de EL1.
    FaultPrivileged,
    /// `brk #0`.
    FaultTrap,
    /// Aponta o `sp` para um valor inválido e usa a pilha: o kernel não pode cair ao voltar.
    FaultBadStack,
    /// Chama `nop` por `x0` ciclos do contador, recebendo IRQ (o par do caso do x86_64).
    FlagsAc,
    /// `x0` chamadas `nop` e sai com os ciclos do contador que levaram.
    NullLoop,
}

fn address(symbol: *const u8) -> usize {
    symbol.addr()
}

/// A imagem do programa.
pub fn image() -> &'static [u8] {
    let start = address(ptr::addr_of!(rustos_user_blob_start));
    let end = address(ptr::addr_of!(rustos_user_blob_end));
    // SAFETY: `[start, end)` é o trecho de `.text` que o `global_asm!` acima delimita.
    unsafe { core::slice::from_raw_parts(ptr::with_exposed_provenance::<u8>(start), end - start) }
}

/// O deslocamento de cada caso dentro da imagem.
pub fn offset(case: Case) -> usize {
    let start = address(ptr::addr_of!(rustos_user_blob_start));
    let at = match case {
        Case::Hello => address(ptr::addr_of!(rustos_user_hello)),
        Case::FaultMemory => address(ptr::addr_of!(rustos_user_fault_mem)),
        Case::FaultPrivileged => address(ptr::addr_of!(rustos_user_fault_priv)),
        Case::FaultTrap => address(ptr::addr_of!(rustos_user_fault_trap)),
        Case::FaultBadStack => address(ptr::addr_of!(rustos_user_fault_stack)),
        Case::FlagsAc => address(ptr::addr_of!(rustos_user_flags)),
        Case::NullLoop => address(ptr::addr_of!(rustos_user_null_loop)),
    };
    at - start
}
