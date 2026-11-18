//! Porta do processador: máscara de interrupção local, espera, estado por CPU e IPI.

/// Núcleos que o kernel guarda estado por CPU; os que o bootloader entregar além
/// disso ficam parados.
pub const MAX_CPUS: usize = 8;

pub trait Cpu {
    fn disable_interrupts(&self);

    /// Dorme até a próxima interrupção; com interrupções mascaradas, não volta.
    fn wait_for_interrupt(&self);

    /// Desliga as interrupções da CPU local e diz se estavam ligadas.
    fn save_and_disable_interrupts(&self) -> bool;

    /// Volta ao estado que `save_and_disable_interrupts` devolveu.
    fn restore_interrupts(&self, were_enabled: bool);

    /// Chamada com interrupções desligadas; volta com elas ligadas e depois de
    /// atender ao menos uma. Uma interrupção que chegue no meio não se perde.
    fn enable_interrupts_and_wait(&self);

    fn halt_forever(&self) -> ! {
        self.disable_interrupts();
        loop {
            self.wait_for_interrupt();
        }
    }
}

/// Estado por CPU lido sem custo: o adaptador guarda o endereço de um bloco por
/// CPU onde a CPU o acha sozinha (`GS` no x86_64, `TPIDR_EL1` no aarch64).
pub trait CpuLocal {
    /// Registra o bloco desta CPU. `block` é o endereço de um bloco que vive para
    /// sempre e é só desta CPU, e o primeiro `usize` dele guarda o próprio `block`
    /// (é o que `gs:[0]` lê no x86_64).
    fn install(&self, block: usize);

    /// O `block` que `install` registrou nesta CPU; só vale depois de `install` nela.
    fn block(&self) -> usize;
}

/// Para que serve uma interrupção entre processadores.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IpiKind {
    /// Refaz a decisão de escalonamento da CPU destino: chegou tarefa ou prazo.
    Reschedule,
    /// Para a CPU destino para sempre: pânico em outro núcleo.
    Stop,
}

/// Interrupção entre processadores (ICR do APIC, SGI do GIC).
pub trait Ipi {
    /// Manda `kind` à CPU de identificador `hw_id` (`CpuDescriptor::hw_id`).
    fn send(&self, hw_id: u64, kind: IpiKind);

    /// Manda `kind` a todas as CPUs menos esta.
    fn send_all_but_self(&self, kind: IpiKind);
}
