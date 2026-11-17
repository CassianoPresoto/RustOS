//! Exceções e interrupções do lado do núcleo: relatório de falha, tique do timer
//! e a fila de entrada que a IRQ enche e o laço do shell esvazia.

use core::sync::atomic::{AtomicU64, Ordering};

use rustos_hal::{BootInfo, Cpu, Fault, FaultKind, Handlers, Irq};
use rustos_sync::ByteRing;

use crate::memory::Memory;
use crate::{console, platform};

/// De onde veio uma IRQ, na visão do núcleo; o `platform` traduz o número.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code, reason = "BASE: os adaptadores de cada ISA constroem")]
pub enum IrqSource {
    Timer,
    Serial,
    Keyboard,
    Spurious,
    Unknown,
}

/// Teste de exceção que o shell e o boot pedem à plataforma.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultTest {
    Breakpoint,
    PageFault,
}

static HANDLERS: Handlers = Handlers {
    fault: on_fault,
    irq: on_irq,
};
static TICKS: AtomicU64 = AtomicU64::new(0);
static BREAKPOINTS: AtomicU64 = AtomicU64::new(0);
/// Produtor: só `on_irq`. Consumidor: só `pop_input`, no laço do shell.
static INPUT: ByteRing<256> = ByteRing::new();

/// Liga exceções, controlador, timer e entrada, e destrava as interrupções da CPU.
/// `false` quando a plataforma ainda não sabe: o kernel segue sem interrupção.
pub fn init(boot: &impl BootInfo, memory: &mut Memory) -> bool {
    match platform::init_interrupts(boot, memory, &HANDLERS) {
        Ok(()) => {
            platform::cpu().restore_interrupts(true);
            true
        }
        Err(error) => {
            kprintln!("[rustos] interrupts: {error:?}");
            false
        }
    }
}

pub fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

pub fn breakpoints() -> u64 {
    BREAKPOINTS.load(Ordering::Relaxed)
}

/// Próximo byte digitado. Chamar só do laço do shell, o único consumidor.
pub fn pop_input() -> Option<u8> {
    // SAFETY: o laço do shell é o único consumidor da fila (contrato desta função).
    unsafe { INPUT.pop() }
}

fn push_input(byte: u8) {
    // SAFETY: `on_irq` é o único produtor e roda com IRQ mascarada, sem reentrar.
    unsafe { INPUT.push(byte) };
}

fn on_fault(fault: &Fault) {
    if fault.kind.is_recoverable() {
        BREAKPOINTS.fetch_add(1, Ordering::Relaxed);
        return;
    }
    console::print_panic(format_args!(
        "[rustos] fault: {} at pc {:#x} (code {:#x})\n",
        Described(fault.kind),
        fault.pc.as_u64(),
        fault.code
    ));
    platform::cpu().halt_forever();
}

fn on_irq(irq: Irq) {
    match platform::irq_source(irq) {
        IrqSource::Timer => {
            TICKS.fetch_add(1, Ordering::Relaxed);
            platform::timer_tick();
        }
        IrqSource::Serial => {
            while let Some(byte) = console::read_byte() {
                push_input(byte);
            }
            platform::serial_irq_done();
        }
        IrqSource::Keyboard => {
            while let Some(event) = platform::keyboard_event() {
                console::key_event(event, push_input);
            }
        }
        IrqSource::Spurious | IrqSource::Unknown => {}
    }
}

struct Described(FaultKind);

impl core::fmt::Display for Described {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.0 {
            FaultKind::PageFault {
                address,
                write,
                execute,
                user,
                present,
            } => write!(
                f,
                "page fault at {:#x} ({}, {}, {})",
                address.as_u64(),
                if execute {
                    "execute"
                } else if write {
                    "write"
                } else {
                    "read"
                },
                if user { "user" } else { "kernel" },
                if present { "protection" } else { "not mapped" },
            ),
            FaultKind::Breakpoint => f.write_str("breakpoint"),
            FaultKind::InvalidOpcode => f.write_str("invalid opcode"),
            FaultKind::DivideError => f.write_str("divide error"),
            FaultKind::GeneralProtection => f.write_str("general protection"),
            FaultKind::Alignment => f.write_str("alignment"),
            FaultKind::DoubleFault => f.write_str("double fault"),
            FaultKind::MachineCheck => f.write_str("machine check"),
            FaultKind::Other => f.write_str("exception"),
        }
    }
}
