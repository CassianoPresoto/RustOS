//! QEMU `virt` aarch64: PL011 e GICv3 por MMIO, mapeados como dispositivo pelo
//! kernel; exceções e timer pelos registradores de sistema.

use core::sync::atomic::{AtomicU8, AtomicU64, Ordering};

pub use rustos_arch_aarch64::call_on_stack;
use rustos_arch_aarch64::intid::{QEMU_VIRT_PL011_INTID, VIRTUAL_TIMER_INTID};
use rustos_arch_aarch64::{
    Aarch64Cpu, Aarch64Entropy, Aarch64PageMapper, MmioBus, QEMU_VIRT_GICD, QEMU_VIRT_GICR,
    QEMU_VIRT_PL011, exception, gic_cpu, intid, protection, timer,
};
use rustos_driver_gicv3::{GicError, GicV3, affinity_from_mpidr, router_for};
use rustos_driver_pl011::Pl011;
use rustos_hal::{
    BootInfo, FrameSource, Handlers, Irq, KeyEvent, MapError, PAGE_SIZE, PhysAddr, VirtAddr,
};

use super::{PlatformError, Protection};
use crate::interrupts::{FaultTest, IrqSource};
use crate::memory::{Memory, map_device};

pub const ARCH: &str = "aarch64";

pub type PlatformConsole = Pl011<MmioBus>;
pub type PlatformEntropy = Aarch64Entropy;
/// Espaço de endereçamento: as tabelas da metade alta atrás do `TTBR1_EL1`.
pub type PlatformSpace = Aarch64PageMapper;

/// Frequência do tique do timer.
pub const TIMER_HZ: u32 = 100;

/// Pilha das exceções: 4 páginas e a de guarda embaixo.
const EXCEPTION_STACK_PAGES: u64 = 4;
/// Registradores do distribuidor (64 KiB) e do redistribuidor da CPU 0 (`RD_base` + `SGI_base`).
const GICD_LEN: u64 = 0x1_0000;
const GICR_LEN: u64 = 0x2_0000;
/// Prioridade do GIC: quanto menor o número, mais urgente.
const TIMER_PRIORITY: u8 = 0x40;
const SERIAL_PRIORITY: u8 = 0x80;

static UART_BASE: AtomicU64 = AtomicU64::new(0);
static STACK_GUARD: AtomicU64 = AtomicU64::new(0);
static PAN_STATE: AtomicU8 = AtomicU8::new(PAN_UNSUPPORTED);
const PAN_UNSUPPORTED: u8 = 0;
const PAN_ON: u8 = 1;
const PAN_OFF: u8 = 2;

/// Liga as proteções de hardware da CPU, antes das tabelas novas.
pub fn harden() {
    if protection::pan_available() {
        // SAFETY: EL1, e a CPU anuncia `PAN`.
        let on = unsafe { protection::enable_pan() };
        PAN_STATE.store(if on { PAN_ON } else { PAN_OFF }, Ordering::Relaxed);
    }
}

pub fn cpu() -> Aarch64Cpu {
    // SAFETY: o Limine entrega o kernel em EL1 (ou EL2 com VHE), nunca em EL0.
    unsafe { Aarch64Cpu::new() }
}

pub fn console(
    boot: &impl BootInfo,
    frames: &mut impl FrameSource,
) -> Result<PlatformConsole, MapError> {
    // SAFETY: MMU ligada; as tabelas do Limine e os frames novos estão no mapa direto.
    let mut mapper = unsafe { Aarch64PageMapper::active(boot.direct_map_offset()) };
    let registers = PhysAddr::new(QEMU_VIRT_PL011);
    let virt = map_device(&mut mapper, frames, registers, PAGE_SIZE)?;
    UART_BASE.store(virt.as_u64(), Ordering::Relaxed);
    // SAFETY: a página acabou de ser mapeada como dispositivo e cobre todo o bloco da PL011.
    let bus = unsafe { MmioBus::new(virt) };
    let mut uart = Pl011::new(bus);
    uart.init();
    Ok(uart)
}

/// Liga exceções, controlador de interrupção, timer em `TIMER_HZ` e a IRQ de
/// recepção da serial. Deixa a máscara da CPU desligada: o chamador decide.
pub fn init_interrupts(
    boot: &impl BootInfo,
    memory: &mut Memory,
    handlers: &'static Handlers,
) -> Result<(), PlatformError> {
    // SAFETY: MMU ligada; as tabelas do Limine e os frames novos estão no mapa direto.
    let mut mapper = unsafe { Aarch64PageMapper::active(boot.direct_map_offset()) };
    let stack_top = memory
        .map_stack(&mut mapper, EXCEPTION_STACK_PAGES)
        .map_err(map_error)?;
    let guard = stack_top.as_u64() - (EXCEPTION_STACK_PAGES + 1) * PAGE_SIZE;
    STACK_GUARD.store(guard, Ordering::Relaxed);
    // SAFETY: EL1 com IRQ mascarada; o topo é o da pilha recém-mapeada, só das exceções.
    unsafe { exception::install(handlers, stack_top) }
        .map_err(|_| PlatformError::MissingFeature("EL1t entry (kernel is not on SP_EL0)"))?;

    let distributor = map_gic(&mut mapper, memory, QEMU_VIRT_GICD, GICD_LEN)?;
    let redistributor = map_gic(&mut mapper, memory, QEMU_VIRT_GICR, GICR_LEN)?;
    // SAFETY: as duas faixas acabaram de ser mapeadas como dispositivo e cobrem os blocos.
    let mut gic = GicV3::new(unsafe { MmioBus::new(distributor) }, unsafe {
        MmioBus::new(redistributor)
    });
    let affinity = affinity_from_mpidr(gic_cpu::mpidr());
    gic.init_distributor().map_err(gic_error)?;
    gic.init_redistributor(affinity).map_err(gic_error)?;
    // SAFETY: EL1 e o redistribuidor desta CPU acabou de acordar.
    unsafe { gic_cpu::enable() }
        .map_err(|_| PlatformError::MissingFeature("GICv3 system registers (ICC_SRE_EL1)"))?;
    gic.enable_ppi(VIRTUAL_TIMER_INTID, TIMER_PRIORITY)
        .map_err(gic_error)?;
    gic.enable_spi(QEMU_VIRT_PL011_INTID, SERIAL_PRIORITY, router_for(affinity))
        .map_err(gic_error)?;
    serial().enable_rx_interrupt();

    // SAFETY: EL1, primeira e única vez nesta CPU.
    unsafe { timer::start(TIMER_HZ) }.map_err(|error| match error {
        timer::TimerError::NoFrequency => PlatformError::MissingFeature("CNTFRQ_EL0"),
        timer::TimerError::PeriodOutOfRange => {
            PlatformError::MissingFeature("timer period (CNTFRQ_EL0 too low)")
        }
    })?;
    Ok(())
}

fn map_gic(
    mapper: &mut Aarch64PageMapper,
    memory: &mut Memory,
    phys: u64,
    len: u64,
) -> Result<VirtAddr, PlatformError> {
    map_device(mapper, &mut memory.frames, PhysAddr::new(phys), len).map_err(map_error)
}

fn map_error(error: MapError) -> PlatformError {
    PlatformError::Map(error)
}

fn gic_error(error: GicError) -> PlatformError {
    PlatformError::MissingFeature(match error {
        GicError::NotGicV3 => "GICv3 (distributor says otherwise)",
        GicError::WrongRedistributor => "GICv3 redistributor of this CPU",
        GicError::Timeout => "GICv3 (hardware did not answer)",
        GicError::InvalidIntid => "GICv3 interrupt line",
    })
}

/// A PL011 de novo sobre a mesma página, só para os registradores de interrupção.
/// Não passa pelo lock do console: a IRQ não espera quem está escrevendo.
fn serial() -> Pl011<MmioBus> {
    let base = UART_BASE.load(Ordering::Relaxed);
    // SAFETY: `console` mapeou a página da PL011 como dispositivo antes de qualquer IRQ.
    Pl011::new(unsafe { MmioBus::new(VirtAddr::new(base)) })
}

/// Traduz o número da IRQ para a fonte que o núcleo entende.
pub fn irq_source(irq: Irq) -> IrqSource {
    match irq.0 {
        VIRTUAL_TIMER_INTID => IrqSource::Timer,
        QEMU_VIRT_PL011_INTID => IrqSource::Serial,
        intid if !intid::needs_end_of_interrupt(intid) => IrqSource::Spurious,
        _ => IrqSource::Unknown,
    }
}

/// Chamado a cada tique, dentro da IRQ: rearma o timer, que é de um disparo só.
pub fn timer_tick() {
    timer::rearm();
}

/// Chamado antes de drenar a UART, dentro da IRQ: limpa o pedido de recepção.
/// O byte que chegar depois rearma a IRQ em vez de ficar parado no FIFO.
pub fn serial_irq_done() {
    serial().clear_rx_interrupt();
}

/// A `virt` não tem teclado nesta fase: a entrada é só a serial.
pub fn has_keyboard() -> bool {
    false
}

/// A `virt` não tem teclado nesta fase: a entrada é só a serial.
pub fn keyboard_event() -> Option<KeyEvent> {
    None
}

/// Nanossegundos desde que `init_interrupts` ligou o relógio; `None` antes.
pub fn now_ns() -> Option<u64> {
    timer::now_ns()
}

/// Contador virtual (`CNTVCT_EL0`): o relógio da semente de reserva.
pub fn cycles() -> u64 {
    timer::counter()
}

/// `RNDRRS` ou `RNDR`, se `ID_AA64ISAR0_EL1.RNDR` anunciar.
pub fn hardware_entropy() -> Option<PlatformEntropy> {
    Aarch64Entropy::detect()
}

/// As proteções de hardware desta ISA e se ficaram ligadas.
pub fn protections() -> &'static [Protection] {
    const ON: [Protection; 1] = [Protection {
        name: "pan",
        enabled: true,
    }];
    const OFF: [Protection; 1] = [Protection {
        name: "pan",
        enabled: false,
    }];
    match PAN_STATE.load(Ordering::Relaxed) {
        PAN_ON => &ON,
        PAN_OFF => &OFF,
        _ => &[],
    }
}

/// Provoca a exceção pedida; `PageFault` lê a página de guarda da pilha de exceção.
pub fn trigger(test: FaultTest) -> bool {
    match test {
        FaultTest::WriteText => {
            crate::vm::probe_write_text();
            true
        }
        FaultTest::Breakpoint => {
            exception::breakpoint();
            true
        }
        FaultTest::PageFault => {
            let guard = STACK_GUARD.load(Ordering::Relaxed);
            if guard != 0 {
                exception::probe_read(VirtAddr::new(guard));
            }
            false
        }
    }
}
