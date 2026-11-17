//! Exceções e interrupções do lado do núcleo: relatório de falha, tique do timer
//! e a fila de entrada que a IRQ enche e o laço do shell esvazia.

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use rustos_hal::{BootInfo, Cpu, Fault, FaultKind, Handlers, Irq};
use rustos_sync::ByteRing;

use crate::memory::Memory;
use crate::{console, platform, task};

/// De onde veio uma IRQ, na visão do núcleo; o `platform` traduz o número.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IrqSource {
    Timer,
    Serial,
    #[cfg_attr(
        target_arch = "aarch64",
        allow(dead_code, reason = "a `virt` não tem teclado")
    )]
    Keyboard,
    Spurious,
    Unknown,
}

/// Teste de exceção que o shell e o boot pedem à plataforma.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultTest {
    Breakpoint,
    PageFault,
    /// Escrita em `.text`: tem de ser um page fault de proteção (W^X).
    WriteText,
}

static HANDLERS: Handlers = Handlers {
    fault: on_fault,
    irq: on_irq,
    preempt: task::preempt,
};
/// Período do tique de boot, antes de o escalonador ligar; depois dele não há tique periódico.
const TICK_NS: u64 = 1_000_000_000 / platform::TIMER_HZ as u64;
/// Prazo do timer quando nada o exige: longe, como o `start` do `CNTV` o deixa.
const PARKED_NS: u64 = 3_600_000_000_000;
/// Interrupções do timer desde o boot.
static TICKS: AtomicU64 = AtomicU64::new(0);
static NEXT_TICK_NS: AtomicU64 = AtomicU64::new(0);
/// O prazo que está no timer de um tiro agora.
static ARMED_NS: AtomicU64 = AtomicU64::new(u64::MAX);
static IN_IRQ: AtomicBool = AtomicBool::new(false);
static BREAKPOINTS: AtomicU64 = AtomicU64::new(0);
/// Produtor: só `on_irq`. Consumidor: só `pop_input`, no laço do shell.
static INPUT: ByteRing<256> = ByteRing::new();

/// Liga exceções, controlador, timer e entrada, e destrava as interrupções da CPU.
/// `false` se a plataforma falhou: sem interrupção o shell não tem entrada.
pub fn init(boot: &impl BootInfo, memory: &mut Memory) -> bool {
    match platform::init_interrupts(boot, memory, &HANDLERS) {
        Ok(()) => {
            let first = platform::now_ns().unwrap_or(0) + TICK_NS;
            NEXT_TICK_NS.store(first, Ordering::Relaxed);
            arm(first);
            platform::cpu().restore_interrupts(true);
            true
        }
        Err(error) => {
            kprintln!("[rustos] interrupts: {error:?}");
            false
        }
    }
}

fn arm(deadline_ns: u64) {
    ARMED_NS.store(deadline_ns, Ordering::Relaxed);
    platform::arm_timer(deadline_ns);
}

/// Adianta o timer de um tiro se `deadline_ns` vem antes do prazo armado.
/// Chamar com a interrupção mascarada.
pub fn arm_no_later_than(deadline_ns: u64) {
    if deadline_ns < ARMED_NS.load(Ordering::Relaxed) {
        arm(deadline_ns);
    }
}

/// Se a CPU está dentro de um handler de IRQ.
pub fn in_irq() -> bool {
    IN_IRQ.load(Ordering::Relaxed)
}

/// IRQ do timer. Antes do escalonador, o tique periódico do autoteste. Depois, só o
/// que ele pede: o fim da fatia da corrente ou o próximo acordar; sem nenhum dos
/// dois o timer fica longe e a CPU ociosa não acorda por ele. A troca vem no fim da IRQ.
fn on_timer() {
    let now = platform::now_ns().unwrap_or(0);
    TICKS.fetch_add(1, Ordering::Relaxed);
    if task::is_ready() {
        match task::timer_irq(now) {
            Some(at) => arm(at),
            None => arm(now + PARKED_NS),
        }
        return;
    }
    let tick_due = NEXT_TICK_NS.load(Ordering::Relaxed);
    NEXT_TICK_NS.store(next_tick(tick_due, now), Ordering::Relaxed);
    arm(NEXT_TICK_NS.load(Ordering::Relaxed));
}

/// O tique seguinte, um período depois do anterior sem acumular atraso; se a IRQ
/// veio tão tarde que esse já passou, um período depois de agora.
const fn next_tick(previous: u64, now: u64) -> u64 {
    let next = previous + TICK_NS;
    if next > now { next } else { now + TICK_NS }
}

/// Interrupções do timer desde o boot.
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

/// Próximo byte digitado; dorme (a tarefa, não a CPU) até a IRQ trazer um.
/// Só o laço do shell chama: ele é o único consumidor da fila.
pub fn read_input_blocking() -> u8 {
    let cpu = platform::cpu();
    loop {
        let were_enabled = cpu.save_and_disable_interrupts();
        if let Some(byte) = pop_input() {
            cpu.restore_interrupts(were_enabled);
            return byte;
        }
        task::wait_for_input();
        cpu.restore_interrupts(were_enabled);
    }
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
    IN_IRQ.store(true, Ordering::Relaxed);
    match platform::irq_source(irq) {
        IrqSource::Timer => on_timer(),
        IrqSource::Serial => {
            platform::serial_irq_done();
            while let Some(byte) = console::read_byte() {
                push_input(byte);
            }
            task::notify_input();
        }
        IrqSource::Keyboard => {
            while let Some(event) = platform::keyboard_event() {
                console::key_event(event, push_input);
            }
            task::notify_input();
        }
        IrqSource::Spurious | IrqSource::Unknown => {}
    }
    IN_IRQ.store(false, Ordering::Relaxed);
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
