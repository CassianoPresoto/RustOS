//! O começo do bloco por CPU que o `asm` de entrada conhece pelo `GS`: o kernel o embute como
//! o primeiro campo do `PerCpu` (SDD 17, "Interação com o SMP e o estado por CPU"). Puro.

use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// `gs:[0]` é o endereço do próprio bloco; depois dele, o que o `SYSCALL` e a troca de tarefa usam.
#[derive(Debug)]
#[repr(C)]
pub struct CpuPrefix {
    pub(crate) block: AtomicUsize,
    /// O `rsp` do usuário, guardado pelo `SYSCALL` até o quadro estar montado.
    pub(crate) user_rsp: AtomicU64,
    /// Topo da pilha do kernel da tarefa de usuário que roda aqui (o `RSP0` da TSS desta CPU).
    pub(crate) kernel_rsp: AtomicU64,
    /// Endereço do `rsp[0]` da TSS desta CPU, para a troca de tarefa o escrever.
    pub(crate) tss_rsp0: AtomicU64,
    /// A raiz de página que o `CR3` desta CPU tem agora (zero antes da primeira troca).
    pub(crate) active_root: AtomicU64,
}

impl CpuPrefix {
    pub const fn new() -> Self {
        Self {
            block: AtomicUsize::new(0),
            user_rsp: AtomicU64::new(0),
            kernel_rsp: AtomicU64::new(0),
            tss_rsp0: AtomicU64::new(0),
            active_root: AtomicU64::new(0),
        }
    }
}

impl CpuPrefix {
    /// Registra o endereço do próprio bloco (o que `gs:[0]` lê).
    pub fn set_block(&self, block: usize) {
        self.block.store(block, Ordering::Relaxed);
    }
}

impl Default for CpuPrefix {
    fn default() -> Self {
        Self::new()
    }
}

/// Offsets que o `asm` de entrada usa em `gs:[…]`.
pub const OFFSET_USER_RSP: usize = core::mem::offset_of!(CpuPrefix, user_rsp);
pub const OFFSET_KERNEL_RSP: usize = core::mem::offset_of!(CpuPrefix, kernel_rsp);

#[cfg(test)]
mod tests {
    use core::mem::{offset_of, size_of};

    use super::*;

    #[test]
    fn the_block_pointer_is_first_and_the_entry_offsets_follow_it() {
        assert_eq!(offset_of!(CpuPrefix, block), 0);
        assert_eq!(OFFSET_USER_RSP, 8);
        assert_eq!(OFFSET_KERNEL_RSP, 16);
        assert_eq!(size_of::<CpuPrefix>(), 40);
    }
}
