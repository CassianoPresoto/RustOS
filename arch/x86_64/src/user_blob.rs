//! O programa de usuário de teste da fatia 4a, embutido no kernel só para o modo de teste
//! (feature `selftest`): o kernel copia `[start, end)` para uma página de usuário. Código
//! independente de posição; cada entrada é um caso do autoteste (SDD 17).

use core::arch::global_asm;
use core::ptr;

use rustos_hal::abi::Syscall;

const HELLO: &str = "hello from ring3\n";

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
    .ascii "hello from ring3\n"
.Lhello_end:

    .balign 16
rustos_user_hello:
    lea rdi, [rip + .Lhello_msg]
    mov esi, {hello_len}
    mov eax, {debug_write}
    syscall
    test rax, rax
    jnz .Lfail1
    mov eax, {clock}
    syscall
    test rax, rax
    jnz .Lfail2
    test rdx, rdx
    jz .Lfail2
    sub rsp, 64
    mov rdi, rsp
    mov esi, 32
    mov eax, {random}
    syscall
    add rsp, 64
    test rax, rax
    jnz .Lfail3
    mov eax, 0x7777
    syscall
    cmp rax, -7
    jne .Lfail4
    mov eax, {thread_exit}
    syscall
    cmp rax, -7
    jne .Lfail4
    mov edi, 0x20000
    mov esi, 8
    mov eax, {debug_write}
    syscall
    cmp rax, -8
    jne .Lfail5
    xor edi, edi
    jmp .Lexit
.Lfail1:
    mov edi, 1
    jmp .Lexit
.Lfail2:
    mov edi, 2
    jmp .Lexit
.Lfail3:
    mov edi, 3
    jmp .Lexit
.Lfail4:
    mov edi, 4
    jmp .Lexit
.Lfail5:
    mov edi, 5
.Lexit:
    mov eax, {process_exit}
    syscall
    ud2

    .balign 16
rustos_user_fault_mem:
    mov rax, [rdi]
    xor edi, edi
    mov eax, {process_exit}
    syscall
    ud2

    .balign 16
rustos_user_fault_priv:
    pushfq
    or qword ptr [rsp], 0x40000
    popfq
    hlt
    xor edi, edi
    mov eax, {process_exit}
    syscall
    ud2

    .balign 16
rustos_user_fault_trap:
    pushfq
    or qword ptr [rsp], 0x100
    popfq
    nop
    nop
    xor edi, edi
    mov eax, {process_exit}
    syscall
    ud2

    .balign 16
rustos_user_fault_stack:
    mov rax, 0x1234800000000000
    mov rsp, rax
    xor eax, eax
    syscall
    xor edi, edi
    mov eax, {process_exit}
    syscall
    ud2

    .balign 16
rustos_user_flags:
    mov r12, rdi
    pushfq
    or qword ptr [rsp], 0x4000
    popfq
    pushfq
    or qword ptr [rsp], 0x40000
    popfq
    pushfq
    pop rax
    and eax, 0x44000
    cmp eax, 0x44000
    jne 3f
    rdtsc
    shl rdx, 32
    or rax, rdx
    mov r13, rax
2:
    xor eax, eax
    syscall
    rdtsc
    shl rdx, 32
    or rax, rdx
    sub rax, r13
    cmp rax, r12
    jb 2b
    xor edi, edi
    jmp 4f
3:
    mov edi, 3
4:
    mov eax, {process_exit}
    syscall
    ud2

    .balign 16
rustos_user_null_loop:
    mov r12, rdi
    rdtsc
    shl rdx, 32
    or rax, rdx
    mov r13, rax
2:
    xor eax, eax
    syscall
    dec r12
    jnz 2b
    rdtsc
    shl rdx, 32
    or rax, rdx
    sub rax, r13
    mov rdi, rax
    mov eax, {process_exit}
    syscall
    ud2
rustos_user_blob_end:
    "#,
    hello_len = const HELLO.len(),
    debug_write = const Syscall::DebugWrite as u64,
    process_exit = const Syscall::ProcessExit as u64,
    clock = const Syscall::ClockGetMonotonic as u64,
    random = const Syscall::RandomGet as u64,
    thread_exit = const Syscall::ThreadExit as u64,
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
    /// Lê o endereço que recebe em `rdi`.
    FaultMemory,
    /// Executa uma instrução privilegiada.
    FaultPrivileged,
    /// Liga o `TF`: a exceção de depuração (a IST4).
    FaultTrap,
    /// Aponta o `sp` para um valor inválido e usa a pilha: o kernel não pode cair ao voltar.
    FaultBadStack,
    /// Liga `NT` e `AC` por `popf` e chama `nop` por `rdi` ciclos de TSC, recebendo IRQ: o kernel
    /// não pode rodar com `AC` ligado nem o processo morrer.
    FlagsAc,
    /// `rdi` chamadas `nop` e sai com os ciclos do TSC que levaram.
    NullLoop,
}

fn address(symbol: *const u8) -> usize {
    symbol.addr()
}

/// A imagem do programa e o deslocamento de cada caso dentro dela.
pub fn image() -> &'static [u8] {
    let start = address(ptr::addr_of!(rustos_user_blob_start));
    let end = address(ptr::addr_of!(rustos_user_blob_end));
    // SAFETY: `[start, end)` é o trecho de `.text` que o `global_asm!` acima delimita.
    unsafe { core::slice::from_raw_parts(ptr::with_exposed_provenance::<u8>(start), end - start) }
}

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
