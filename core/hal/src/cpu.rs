//! Porta do processador: só o que o núcleo usa hoje.

pub trait Cpu {
    fn disable_interrupts(&self);

    /// Dorme até a próxima interrupção; com interrupções mascaradas, não volta.
    fn wait_for_interrupt(&self);

    fn halt_forever(&self) -> ! {
        self.disable_interrupts();
        loop {
            self.wait_for_interrupt();
        }
    }
}
