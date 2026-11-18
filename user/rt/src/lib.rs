//! Runtime dos programas de usuário (regra 11 do guidelines 03): lê a pilha inicial, chama o
//! `main` do programa, vira as syscalls em funções seguras e cuida de pânico e alocação. O único
//! `unsafe` do lado do usuário mora aqui, com `// SAFETY:`.

#![cfg_attr(not(test), no_std)]
#![allow(
    clippy::cast_possible_truncation,
    reason = "o usuário só existe em 64 bits"
)]

mod console;
mod env;
#[cfg_attr(
    not(target_os = "none"),
    allow(dead_code, reason = "no host só os testes usam o alocador")
)]
mod heap;
#[cfg(target_os = "none")]
pub mod sys;

pub use console::{LineBuffer, print_line};
pub use env::{Auxv, Env};
pub use rustos_hal::abi::Status;

/// Declara o `main` do programa: `fn(&Env) -> i64`, cujo valor é o código de `process_exit`.
#[macro_export]
macro_rules! entry {
    ($main:path) => {
        #[unsafe(no_mangle)]
        pub fn __rustos_main(env: &$crate::Env) -> i64 {
            $main(env)
        }
    };
}

/// Escreve uma linha no console de depuração (`[user] ` na frente, por conta do kernel).
#[macro_export]
macro_rules! println {
    ($($arg:tt)*) => {
        $crate::print_line(::core::format_args!($($arg)*))
    };
}

#[cfg(all(target_os = "none", not(test)))]
unsafe extern "Rust" {
    fn __rustos_main(env: &Env) -> i64;
}

/// O ponto de entrada em Rust, chamado pelo `_start`: lê a pilha, roda o `main` e sai com o código.
///
/// # Safety
/// `stack` é a pilha inicial que o kernel monta (`abi::InitialStack`); só o `_start` chama.
#[cfg(all(target_os = "none", not(test)))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rustos_user_main(stack: *const u64, boot_handle: u64) -> ! {
    // SAFETY: o contrato da função: a pilha é a que o kernel montou.
    let env = unsafe { Env::from_stack(stack, boot_handle) };
    // SAFETY: o programa define `__rustos_main` com `entry!`.
    let code = unsafe { __rustos_main(&env) };
    sys::process_exit(code)
}

#[cfg(all(target_os = "none", not(test)))]
#[panic_handler]
fn panic(info: &core::panic::PanicInfo<'_>) -> ! {
    println!("panic: {info}");
    sys::process_exit(101)
}

/// O endereço do `_start`, que o kernel põe em `AT_ENTRY`.
#[cfg(target_os = "none")]
pub fn entry_address() -> u64 {
    unsafe extern "C" {
        safe fn _start() -> !;
    }
    _start as *const () as usize as u64
}
