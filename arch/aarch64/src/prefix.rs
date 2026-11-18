//! O começo do bloco por CPU que o `asm` das exceções conhece pelo `TPIDR_EL1`: o kernel o embute
//! como o primeiro campo do `PerCpu` (SDD 17, "Interação com o SMP e o estado por CPU"). Puro.

use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// O primeiro `usize` é o endereço do próprio bloco; depois dele, o que as exceções e a troca de
/// espaço de endereçamento usam.
#[derive(Debug)]
#[repr(C)]
pub struct CpuPrefix {
    pub(crate) block: AtomicUsize,
    /// Topo da pilha de falhas desta CPU; zero até `exception::install`, quando o vetor usa a
    /// pilha que o `SP_EL0` já tem (a precoce).
    pub(crate) fault_stack_top: AtomicU64,
    /// A raiz que o `TTBR0_EL1` desta CPU tem agora (zero antes da primeira troca).
    pub(crate) active_root: AtomicU64,
}

impl CpuPrefix {
    pub const fn new() -> Self {
        Self {
            block: AtomicUsize::new(0),
            fault_stack_top: AtomicU64::new(0),
            active_root: AtomicU64::new(0),
        }
    }

    /// Registra o endereço do próprio bloco (o que o `TPIDR_EL1` aponta).
    pub fn set_block(&self, block: usize) {
        self.block.store(block, Ordering::Relaxed);
    }
}

impl Default for CpuPrefix {
    fn default() -> Self {
        Self::new()
    }
}

/// Offset de `fault_stack_top`, que o `asm` do vetor de falha lê.
pub const OFFSET_FAULT_STACK_TOP: usize = core::mem::offset_of!(CpuPrefix, fault_stack_top);

#[cfg(test)]
mod tests {
    use core::mem::{offset_of, size_of};

    use super::*;

    #[test]
    fn the_block_pointer_is_first_and_the_fault_stack_follows_it() {
        assert_eq!(offset_of!(CpuPrefix, block), 0);
        assert_eq!(OFFSET_FAULT_STACK_TOP, 8);
        assert_eq!(size_of::<CpuPrefix>(), 24);
    }
}
