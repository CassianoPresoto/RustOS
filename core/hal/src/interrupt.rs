//! Porta de exceções e interrupções: o que o adaptador de ISA entrega ao núcleo
//! quando a CPU é desviada. O kernel instala os `Handlers`; o `arch/` os chama.

use crate::VirtAddr;

/// Número da interrupção no controlador: vetor da IDT (x86_64) ou INTID do GIC (aarch64).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Irq(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultKind {
    /// Acesso a endereço sem mapeamento ou sem permissão.
    PageFault {
        address: VirtAddr,
        write: bool,
        execute: bool,
        user: bool,
        present: bool,
    },
    Breakpoint,
    InvalidOpcode,
    DivideError,
    GeneralProtection,
    Alignment,
    DoubleFault,
    MachineCheck,
    /// Qualquer outra: o código cru (vetor no x86_64, classe do `ESR` no aarch64).
    Other,
}

impl FaultKind {
    /// Só o breakpoint deixa a execução continuar; o resto para o kernel.
    pub const fn is_recoverable(self) -> bool {
        matches!(self, Self::Breakpoint)
    }
}

/// Exceção já decodificada. `code` é o cru da ISA: vetor e código de erro no
/// x86_64 (`vector << 32 | error`), `ESR_EL1` no aarch64.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fault {
    pub kind: FaultKind,
    pub pc: VirtAddr,
    pub code: u64,
}

/// O que o `arch/` chama. `fault` volta só para exceção recuperável; `irq` roda
/// com interrupções mascaradas, e o `arch/` sinaliza o fim (EOI) depois dele.
#[derive(Clone, Copy, Debug)]
pub struct Handlers {
    pub fault: fn(&Fault),
    pub irq: fn(Irq),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_breakpoint_is_recoverable() {
        assert!(FaultKind::Breakpoint.is_recoverable());
        assert!(!FaultKind::DoubleFault.is_recoverable());
        let page = FaultKind::PageFault {
            address: VirtAddr::new(0),
            write: false,
            execute: false,
            user: false,
            present: false,
        };
        assert!(!page.is_recoverable());
    }
}
