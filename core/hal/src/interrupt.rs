//! Porta de exceções e interrupções: o que o adaptador de ISA entrega ao núcleo
//! quando a CPU é desviada. O kernel instala os `Handlers`; o `arch/` os chama.

use crate::VirtAddr;
use crate::abi::{SyscallArgs, SyscallReturn};

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
    /// A exceção veio do anel 3 ou de EL0: o processo morre e o kernel segue.
    pub from_user: bool,
    /// Valor do `sp` do usuário quando `from_user`; zero se veio do kernel.
    pub user_sp: u64,
}

/// O que o `arch/` chama. `fault` volta só para exceção recuperável ou para falta de página de
/// usuário que o núcleo resolveu (`from_user`: a instrução se repete); `copy_fault` tenta resolver a
/// falta de página de usuário que a cópia usuário↔kernel provocou e diz se a cópia se repete. `irq`
/// roda mascarada e o `arch/` dá o EOI depois. `preempt` roda depois do EOI, na pilha da tarefa interrompida: ali o
/// núcleo pode trocar de tarefa. `syscall` atende o usuário, mascarada. `leave_user` roda logo
/// antes da volta ao usuário, com o quadro salvo: o núcleo limpa a pilha abaixo dele.
#[derive(Clone, Copy, Debug)]
pub struct Handlers {
    pub fault: fn(&Fault),
    /// `(endereço, escrita)` da falta na cópia: `true` se resolveu e a cópia se repete.
    pub copy_fault: fn(u64, bool) -> bool,
    pub irq: fn(Irq),
    pub preempt: fn(),
    pub syscall: fn(&SyscallArgs) -> SyscallReturn,
    pub leave_user: fn(usize),
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

    #[test]
    fn a_fault_remembers_where_it_came_from() {
        let fault = Fault {
            kind: FaultKind::InvalidOpcode,
            pc: VirtAddr::new(0x40_0000),
            code: 6 << 32,
            from_user: true,
            user_sp: 0x7000_0000,
        };
        assert!(fault.from_user && !fault.kind.is_recoverable());
    }
}
