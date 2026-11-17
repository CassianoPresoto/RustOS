//! Escolha dos adaptadores por ISA. Cada módulo expõe a mesma superfície:
//! `ARCH`, `PlatformConsole`, `cpu()` e `console()`.

#[cfg(target_arch = "aarch64")]
mod aarch64;
#[cfg(target_arch = "x86_64")]
mod x86_64;

#[cfg(target_arch = "aarch64")]
pub use aarch64::{ARCH, PlatformConsole, console, cpu};
#[cfg(target_arch = "x86_64")]
pub use x86_64::{ARCH, PlatformConsole, console, cpu};
