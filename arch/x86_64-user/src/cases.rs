//! Os casos de falha do `user/tests/cases` (SDD 17, fatia 4a), em x86_64: cada função é um caso.
//! Seguras por fora: o valor recebido (endereço ou `sp`) só age como falta que mata o processo, ou
//! como a saída pela syscall de saída. Nenhuma função volta.

use core::arch::asm;

use rustos_hal::abi::Syscall;

/// A linha que `hello` escreve; o `debug_write` confere o tamanho dela.
const HELLO: &[u8] = b"hello from ring3\n";

/// Escreve `HELLO` no console, confere cada syscall e sai com 0 ou com o número do passo que falhou.
pub fn hello(console: u64) -> ! {
    // SAFETY: o caso sai pela syscall de saída; `console` é um número, que o kernel confere, e a
    // mensagem é a constante `HELLO`. O `sp` volta antes de qualquer falha.
    unsafe {
        asm!(
            "mov rdi, r12",
            "mov rsi, r13",
            "mov edx, {hello_len}",
            "mov eax, {debug_write}",
            "syscall",
            "test rax, rax",
            "jnz 2f",
            "mov eax, {clock}",
            "syscall",
            "test rax, rax",
            "jnz 3f",
            "test rdx, rdx",
            "jz 3f",
            "sub rsp, 64",
            "mov rdi, rsp",
            "mov esi, 32",
            "mov eax, {random}",
            "syscall",
            "add rsp, 64",
            "test rax, rax",
            "jnz 4f",
            "mov eax, 0x7777",
            "syscall",
            "cmp rax, -7",
            "jne 5f",
            "xor edi, edi",
            "mov eax, {process_kill}",
            "syscall",
            "cmp rax, -2",
            "jne 5f",
            "mov rdi, r12",
            "mov esi, 0x20000",
            "mov edx, 8",
            "mov eax, {debug_write}",
            "syscall",
            "cmp rax, -8",
            "jne 6f",
            "xor edi, edi",
            "jmp 9f",
            "2:",
            "mov edi, 1",
            "jmp 9f",
            "3:",
            "mov edi, 2",
            "jmp 9f",
            "4:",
            "mov edi, 3",
            "jmp 9f",
            "5:",
            "mov edi, 4",
            "jmp 9f",
            "6:",
            "mov edi, 5",
            "9:",
            "mov eax, {process_exit}",
            "syscall",
            "ud2",
            in("r12") console,
            in("r13") HELLO.as_ptr(),
            hello_len = const HELLO.len(),
            debug_write = const Syscall::DebugWrite as u64,
            clock = const Syscall::ClockGetMonotonic as u64,
            random = const Syscall::RandomGet as u64,
            process_kill = const Syscall::ProcessKill as u64,
            process_exit = const Syscall::ProcessExit as u64,
            options(noreturn),
        )
    }
}

/// Lê `addr`: a falta de página é do processo, que morre com ela.
pub fn read(addr: u64) -> ! {
    // SAFETY: `addr` é um número; a leitura falta (ou não) no próprio processo, e a saída só vem se
    // ela não falta, então nada do chamador é tocado.
    unsafe {
        asm!(
            "mov rax, [rdi]",
            "xor edi, edi",
            "mov eax, {process_exit}",
            "syscall",
            "ud2",
            in("rdi") addr,
            process_exit = const Syscall::ProcessExit as u64,

            options(noreturn),
        )
    }
}

/// Executa `hlt` em anel 3: a exceção geral de proteção mata o processo.
pub fn privileged() -> ! {
    // SAFETY: a instrução privilegiada falta no anel 3 e o processo morre; a saída é o caminho que
    // não deve ser alcançado.
    unsafe {
        asm!(
            "pushfq",
            "or qword ptr [rsp], 0x40000",
            "popfq",
            "hlt",
            "xor edi, edi",
            "mov eax, {process_exit}",
            "syscall",
            "ud2",
            process_exit = const Syscall::ProcessExit as u64,

            options(noreturn),
        )
    }
}

/// Liga o `TF` por `popfq` e executa dois `nop`: a exceção de depuração mata o processo.
pub fn trap() -> ! {
    // SAFETY: o `TF` faz a exceção de depuração do processo, que morre; a saída só vem se ela não
    // acontece.
    unsafe {
        asm!(
            "pushfq",
            "or qword ptr [rsp], 0x100",
            "popfq",
            "nop",
            "nop",
            "xor edi, edi",
            "mov eax, {process_exit}",
            "syscall",
            "ud2",
            process_exit = const Syscall::ProcessExit as u64,

            options(noreturn),
        )
    }
}

/// Aponta `rsp` para `sp`, um endereço inválido, e faz uma syscall: o kernel não pode cair ao voltar.
pub fn bad_stack(sp: u64) -> ! {
    // SAFETY: o `rsp` inválido só vale para o processo, que morre na syscall (ou no uso da pilha);
    // o bloco não volta, então o `rsp` antigo não precisa ser restaurado.
    unsafe {
        asm!(
            "mov rsp, rax",
            "xor eax, eax",
            "syscall",
            "xor edi, edi",
            "mov eax, {process_exit}",
            "syscall",
            "ud2",
            in("rax") sp,
            process_exit = const Syscall::ProcessExit as u64,
            options(noreturn),
        )
    }
}

/// Liga `NT` e `AC` por `popfq` e faz `nop` por `count` ciclos de TSC, recebendo IRQ no meio.
pub fn flags(count: u64) -> ! {
    // SAFETY: o caso só mexe em flags do próprio processo e em syscalls de saída; não toca memória
    // do chamador, e o processo sai pelo `process_exit` com 0 ou 3.
    unsafe {
        asm!(
            "mov r12, rdi",
            "pushfq",
            "or qword ptr [rsp], 0x4000",
            "popfq",
            "pushfq",
            "or qword ptr [rsp], 0x40000",
            "popfq",
            "pushfq",
            "pop rax",
            "and eax, 0x44000",
            "cmp eax, 0x44000",
            "jne 3f",
            "rdtsc",
            "shl rdx, 32",
            "or rax, rdx",
            "mov r13, rax",
            "2:",
            "xor eax, eax",
            "syscall",
            "rdtsc",
            "shl rdx, 32",
            "or rax, rdx",
            "sub rax, r13",
            "cmp rax, r12",
            "jb 2b",
            "xor edi, edi",
            "jmp 4f",
            "3:",
            "mov edi, 3",
            "4:",
            "mov eax, {process_exit}",
            "syscall",
            "ud2",
            in("rdi") count,
            process_exit = const Syscall::ProcessExit as u64,

            options(noreturn),
        )
    }
}

/// Faz `count` chamadas `nop` e sai com os ciclos do TSC que levaram: o laço é a medida da syscall.
pub fn null_loop(count: u64) -> ! {
    // SAFETY: o laço só faz `nop` e syscalls de número 0; a medida é lida dentro do processo, e a
    // saída leva o número de ciclos como código.
    unsafe {
        asm!(
            "mov r12, rdi",
            "rdtsc",
            "shl rdx, 32",
            "or rax, rdx",
            "mov r13, rax",
            "2:",
            "xor eax, eax",
            "syscall",
            "dec r12",
            "jnz 2b",
            "rdtsc",
            "shl rdx, 32",
            "or rax, rdx",
            "sub rax, r13",
            "mov rdi, rax",
            "mov eax, {process_exit}",
            "syscall",
            "ud2",
            in("rdi") count,
            process_exit = const Syscall::ProcessExit as u64,

            options(noreturn),
        )
    }
}
