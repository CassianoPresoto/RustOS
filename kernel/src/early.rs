//! Saída do pânico e da exceção antes de o console existir, e quando o lock dele está preso:
//! escreve direto na serial, sem lock, sem memória e sem tela (SDD 04).

use core::fmt;

use rustos_hal::{Console, Cpu, Fault, Handlers, Irq};

use crate::console::Writer;
use crate::interrupts::FaultLine;
use crate::{addr, platform};

/// Vetores de exceção do começo do boot: imprime na serial e para.
static HANDLERS: Handlers = Handlers {
    fault: on_fault,
    irq: ignore_irq,
    preempt: ignore_preempt,
};

/// Primeira coisa do `kmain`: dali em diante exceção e pânico já saem na serial.
pub fn init(scratch: &mut platform::EarlyScratch) {
    platform::early_init(&HANDLERS, scratch);
}

/// Escreve `args` na serial direto. Nada acontece se a plataforma ainda não tem uma.
pub fn print(args: fmt::Arguments<'_>) {
    if let Some(mut serial) = platform::early_console() {
        addr::scrubbed(Writer(&mut serial), |out| {
            out.write_fmt(args).ok();
        });
    }
}

/// Bytes crus, sem formatar: o que sobra quando até `print` pode ter falhado.
pub fn print_str(text: &str) {
    if let Some(mut serial) = platform::early_console() {
        serial.write_bytes(text.as_bytes());
    }
}

fn on_fault(fault: &Fault) {
    if fault.kind.is_recoverable() {
        return;
    }
    print(format_args!("{}", FaultLine(fault)));
    platform::cpu().halt_forever();
}

fn ignore_irq(_irq: Irq) {}

fn ignore_preempt() {}

/// Endereço de usuário sem mapa em nenhuma fase do boot (o mapa de 1:1 da serial cobre só o 1 GiB do zero).
#[cfg(feature = "panic-tests")]
const UNMAPPED: usize = 0x7000_0000_0000;

/// Ponto de falha que a linha de comando pede (`rustos.panic_early`), só no build de teste.
#[cfg(feature = "panic-tests")]
pub fn test_point(params: rustos_hal::BootParams, here: rustos_hal::EarlyPanic) {
    use rustos_hal::EarlyPanic;

    if params.early_panic != Some(here) {
        return;
    }
    match here {
        EarlyPanic::Fault => {
            let unmapped = core::ptr::with_exposed_provenance::<u8>(UNMAPPED);
            // SAFETY: de propósito: o endereço não está mapeado e a leitura gera o page fault.
            let value = unsafe { unmapped.read_volatile() };
            panic!("panic_early: the read returned {value}");
        }
        EarlyPanic::Entry | EarlyPanic::Activated => panic!("panic_early: {here:?}"),
    }
}
