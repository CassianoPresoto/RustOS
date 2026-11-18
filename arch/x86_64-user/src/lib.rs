//! Adaptador de usuário do x86_64 (regra 12 do guidelines 03): o `_start`, que entrega a pilha
//! inicial e o handle de boot ao `user/rt`, e o stub de `syscall`. Todo `asm!` do usuário mora aqui.

#![no_std]

#[cfg(all(target_arch = "x86_64", target_os = "none"))]
mod entry;
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
mod syscall;

#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub use syscall::syscall;
