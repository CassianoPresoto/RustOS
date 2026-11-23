//! Os casos de falha do `user/tests/cases` (SDD 17, fatia 4a), em aarch64: cada função é um caso.
//! Seguras por fora: o valor recebido (endereço ou `sp`) só age como falta que mata o processo, ou
//! como a saída pela syscall de saída. Nenhuma função volta.

use core::arch::asm;

use rustos_hal::abi::Syscall;

/// A linha que `hello` escreve; o `debug_write` confere o tamanho dela.
const HELLO: &[u8] = b"hello from el0\n";

/// Escreve `HELLO` no console, confere cada syscall e sai com 0 ou com o número do passo que falhou.
pub fn hello(console: u64) -> ! {
    // SAFETY: o caso sai pela syscall de saída; `console` é um número, que o kernel confere, e a
    // mensagem é a constante `HELLO`. O `sp` volta antes de qualquer falha.
    unsafe {
        asm!(
            "mov x0, x20",
            "mov x1, x21",
            "mov x2, #{hello_len}",
            "mov x8, #{debug_write}",
            "svc #0",
            "cbnz x0, 1f",
            "mov x8, #{clock}",
            "svc #0",
            "cbnz x0, 2f",
            "cbz x1, 2f",
            "sub sp, sp, #64",
            "mov x0, sp",
            "mov x1, #32",
            "mov x8, #{random}",
            "svc #0",
            "add sp, sp, #64",
            "cbnz x0, 3f",
            "mov x8, #0x7777",
            "svc #0",
            "cmn x0, #7",
            "b.ne 4f",
            "mov x0, #0",
            "mov x8, #{process_kill}",
            "svc #0",
            "cmn x0, #2",
            "b.ne 4f",
            "mov x0, x20",
            "mov x1, #0x20000",
            "mov x2, #8",
            "mov x8, #{debug_write}",
            "svc #0",
            "cmn x0, #8",
            "b.ne 5f",
            "mov x0, #0",
            "b 9f",
            "1:",
            "mov x0, #1",
            "b 9f",
            "2:",
            "mov x0, #2",
            "b 9f",
            "3:",
            "mov x0, #3",
            "b 9f",
            "4:",
            "mov x0, #4",
            "b 9f",
            "5:",
            "mov x0, #5",
            "9:",
            "mov x8, #{process_exit}",
            "svc #0",
            "brk #0",
            in("x20") console,
            in("x21") HELLO.as_ptr(),
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
            "ldr x1, [x0]",
            "mov x0, #0",
            "mov x8, #{process_exit}",
            "svc #0",
            "brk #0",
            in("x0") addr,
            process_exit = const Syscall::ProcessExit as u64,

            options(noreturn),
        )
    }
}

/// Escreve `ttbr0_el1` em EL0: o registrador é do kernel, e o processo morre na instrução.
pub fn privileged() -> ! {
    // SAFETY: a escrita de registrador de sistema falta em EL0 e o processo morre; a saída é o
    // caminho que não deve ser alcançado.
    unsafe {
        asm!(
            "msr ttbr0_el1, xzr",
            "mov x0, #0",
            "mov x8, #{process_exit}",
            "svc #0",
            "brk #0",
            process_exit = const Syscall::ProcessExit as u64,

            options(noreturn),
        )
    }
}

/// Executa `brk #0`: a exceção de depuração mata o processo.
pub fn trap() -> ! {
    // SAFETY: o `brk` leva o processo à exceção, que o mata; a saída só vem se ela não acontece.
    unsafe {
        asm!(
            "brk #0",
            "mov x0, #0",
            "mov x8, #{process_exit}",
            "svc #0",
            "brk #0",
            process_exit = const Syscall::ProcessExit as u64,

            options(noreturn),
        )
    }
}

/// Aponta `sp` para `sp`, um endereço inválido, e lê a pilha: o kernel não pode cair ao voltar.
pub fn bad_stack(sp: u64) -> ! {
    // SAFETY: o `sp` inválido só vale para o processo, que morre na leitura da pilha; o bloco não
    // volta, então o `sp` antigo não precisa ser restaurado.
    unsafe {
        asm!(
            "mov sp, x1",
            "ldr x0, [sp]",
            "mov x0, #0",
            "mov x8, #{process_exit}",
            "svc #0",
            "brk #0",
            in("x1") sp,
            process_exit = const Syscall::ProcessExit as u64,
            options(noreturn),
        )
    }
}

/// Faz `nop` (syscall nula) até o contador virtual andar `count` ticks, recebendo IRQ no meio.
pub fn flags(count: u64) -> ! {
    // SAFETY: o caso só lê o contador e faz syscalls de número 0; não toca memória do chamador, e a
    // saída leva 0 pelo `process_exit`.
    unsafe {
        asm!(
            "mov x9, x0",
            "mrs x10, cntvct_el0",
            "1:",
            "mov x8, #0",
            "svc #0",
            "mrs x11, cntvct_el0",
            "sub x11, x11, x10",
            "cmp x11, x9",
            "b.lo 1b",
            "mov x0, #0",
            "mov x8, #{process_exit}",
            "svc #0",
            "brk #0",
            in("x0") count,
            process_exit = const Syscall::ProcessExit as u64,

            options(noreturn),
        )
    }
}

/// Faz `count` chamadas `nop` e sai com os ticks do contador que levaram: o laço é a medida da syscall.
pub fn null_loop(count: u64) -> ! {
    // SAFETY: o laço só faz syscalls de número 0 e lê o contador; a medida é lida dentro do processo,
    // e a saída leva o número de ticks como código.
    unsafe {
        asm!(
            "mov x9, x0",
            "mrs x10, cntvct_el0",
            "1:",
            "mov x8, #0",
            "svc #0",
            "subs x9, x9, #1",
            "b.ne 1b",
            "mrs x11, cntvct_el0",
            "sub x0, x11, x10",
            "mov x8, #{process_exit}",
            "svc #0",
            "brk #0",
            in("x0") count,
            process_exit = const Syscall::ProcessExit as u64,

            options(noreturn),
        )
    }
}
