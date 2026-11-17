//! Porta do processador: máscara de interrupção local e espera.

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
