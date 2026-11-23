//! Adaptador de usuário do aarch64 (regra 12 do guidelines 03): o `_start`, que entrega a pilha
//! inicial e o handle de boot ao `user/rt`, e o stub de `svc`. Todo `asm!` do usuário mora aqui.

#![no_std]

#[cfg(all(target_arch = "aarch64", target_os = "none"))]
mod entry;
#[cfg(all(target_arch = "aarch64", target_os = "none"))]
mod simd;
#[cfg(all(target_arch = "aarch64", target_os = "none"))]
mod syscall;

#[cfg(all(target_arch = "aarch64", target_os = "none"))]
pub use syscall::syscall;

#[cfg(all(target_arch = "aarch64", target_os = "none"))]
pub use simd::simd_hold;

#[cfg(all(target_arch = "aarch64", target_os = "none", feature = "test-cases"))]
pub mod cases;
