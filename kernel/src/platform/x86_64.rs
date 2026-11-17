//! PC x86_64: COM1 por porta de E/S, GDT/IDT próprias, Local APIC, I/O APIC e
//! relógio pelo TSC calibrado contra o PIT.

use core::cell::UnsafeCell;
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use rustos_arch_x86_64::clock::elapsed_ns;
use rustos_arch_x86_64::ioapic::{IOAPIC_BASE, IoApic};
use rustos_arch_x86_64::lapic::{VECTOR_KEYBOARD, VECTOR_SERIAL, VECTOR_SPURIOUS, VECTOR_TIMER};
use rustos_arch_x86_64::pic;
use rustos_arch_x86_64::pit::{self, Pit, Reading};
use rustos_arch_x86_64::{
    ApicAccess, ApicMode, COM1, IstStacks, MmioBus, PIC_MASTER, PIC_SLAVE, PIT_PORTS, PortBus,
    Protections, X86Cpu, X86PageMapper, breakpoint, cpu_features, detect_apic, enable_protections,
    install_traps, publish_apic, rdtsc,
};
use rustos_driver_i8042::I8042;
use rustos_driver_uart16550::{DIVISOR_115200, Uart16550};
use rustos_hal::{
    BootInfo, FrameSource, Handlers, Irq, KeyEvent, MapError, PAGE_SIZE, PageMapper, PhysAddr,
};
use rustos_sync::SpinLock;

use super::{PlatformError, Protection};
use crate::interrupts::{FaultTest, IrqSource};
use crate::memory::{Memory, map_device};

pub const ARCH: &str = "x86_64";

pub type PlatformConsole = Uart16550<PortBus>;

/// Frequência do tique do timer.
pub const TIMER_HZ: u32 = 100;

/// Páginas de cada pilha de exceção (#DF, NMI, #MC); a de guarda fica abaixo.
const IST_STACK_PAGES: u64 = 4;
/// Janela de calibração: 20 ms de PIT (1 193 182 Hz × 0,02 s).
const PIT_WINDOW_TICKS: u16 = 23_864;
/// Consultas à porta B antes de desistir do PIT.
const PIT_MAX_POLLS: u32 = 50_000_000;
/// Pinos do I/O APIC da QEMU `q35`: as ISA 1 (teclado) e 4 (COM1) vão direto,
/// sem redirecionamento do MADT (DEBT-0009).
const IOAPIC_PIN_KEYBOARD: u8 = 1;
const IOAPIC_PIN_SERIAL: u8 = 4;
/// Base das portas do controlador PS/2: dado em 0x60, status e comando em 0x64.
const I8042_PORTS: u16 = 0x60;

/// Só a IRQ do teclado lê depois do boot: não há outro dono para disputar.
static KEYBOARD: SpinLock<Option<I8042<PortBus>>> = SpinLock::new(None);

static READY: AtomicBool = AtomicBool::new(false);
/// Primeira página, sem mapeamento, da pilha de #DF: lê-la provoca um #PF.
static GUARD_PAGE: AtomicU64 = AtomicU64::new(0);
static TSC_HZ: AtomicU64 = AtomicU64::new(0);
static TSC_ORIGIN: AtomicU64 = AtomicU64::new(0);

/// Escrita só em `init_interrupts`, antes de qualquer leitura.
struct Report(UnsafeCell<[Protection; 5]>);

// SAFETY: uma CPU, e a única escrita roda no boot com as interrupções mascaradas.
unsafe impl Sync for Report {}

static REPORT: Report = Report(UnsafeCell::new([
    Protection {
        name: "wp",
        enabled: false,
    },
    Protection {
        name: "nx",
        enabled: false,
    },
    Protection {
        name: "smep",
        enabled: false,
    },
    Protection {
        name: "smap",
        enabled: false,
    },
    Protection {
        name: "umip",
        enabled: false,
    },
]));

pub fn cpu() -> X86Cpu {
    // SAFETY: o Limine entrega o kernel em ring 0, e ele nunca sai de lá.
    unsafe { X86Cpu::new() }
}

#[allow(
    clippy::unnecessary_wraps,
    reason = "mesma assinatura em todas as ISAs"
)]
pub fn console(
    _boot: &impl BootInfo,
    _frames: &mut impl FrameSource,
) -> Result<PlatformConsole, MapError> {
    // SAFETY: a COM1 é a UART padrão do PC e só o console a usa.
    let bus = unsafe { PortBus::new(COM1) };
    let mut uart = Uart16550::new(bus);
    uart.init(DIVISOR_115200);
    Ok(uart)
}

/// Liga exceções, controlador de interrupção, timer em `TIMER_HZ` e a IRQ de
/// recepção da serial. Deixa a máscara da CPU desligada: o chamador decide.
pub fn init_interrupts(
    boot: &impl BootInfo,
    memory: &mut Memory,
    handlers: &'static Handlers,
) -> Result<(), PlatformError> {
    // SAFETY: ring 0 no boot, e o kernel não toca memória de usuário.
    publish_protections(unsafe { enable_protections() });
    let features = cpu_features();
    if !features.tsc {
        return Err(PlatformError::MissingFeature("tsc"));
    }
    let mode = detect_apic(features).ok_or(PlatformError::MissingFeature("apic"))?;
    // SAFETY: o mapa direto cobre as tabelas e os frames; só o boot mexe nelas agora.
    let mut mapper = unsafe { X86PageMapper::active(boot.direct_map_offset()) };

    let stacks = IstStacks {
        double_fault: ist_stack(memory, &mut mapper)?,
        nmi: ist_stack(memory, &mut mapper)?,
        machine_check: ist_stack(memory, &mut mapper)?,
    };
    let guard = stacks.double_fault.as_u64() - (IST_STACK_PAGES + 1) * PAGE_SIZE;
    GUARD_PAGE.store(guard, Ordering::Relaxed);
    // SAFETY: ring 0 no boot com IF desligado; as pilhas vêm de `map_stack`.
    unsafe { install_traps(stacks, handlers) };
    READY.store(true, Ordering::Release);

    mask_legacy_pic();
    let (apic, mmio) = enable_local_apic(mode, memory, &mut mapper)?;
    let destination = u8::try_from(apic.local().id(apic.is_x2apic()))
        .map_err(|_| PlatformError::MissingFeature("8-bit apic id"))?;
    let keyboard = start_keyboard();
    route_io_apic(memory, &mut mapper, destination, keyboard)?;
    publish_apic(apic, mmio);
    start_clocks(&apic)?;

    // SAFETY: a COM1 é só do console; este é o boot, com IF desligado.
    let mut uart = Uart16550::new(unsafe { PortBus::new(COM1) });
    uart.enable_receive_interrupt();
    Ok(())
}

fn publish_protections(on: Protections) {
    let enabled = [on.wp, on.nx, on.smep, on.smap, on.umip];
    // SAFETY: única escrita, no boot, antes de `protections` ser lida.
    let report = unsafe { &mut *REPORT.0.get() };
    for (entry, enabled) in report.iter_mut().zip(enabled) {
        entry.enabled = enabled;
    }
}

fn ist_stack(
    memory: &mut Memory,
    mapper: &mut impl PageMapper,
) -> Result<rustos_hal::VirtAddr, PlatformError> {
    memory
        .map_stack(mapper, IST_STACK_PAGES)
        .map_err(PlatformError::Map)
}

fn mask_legacy_pic() {
    // SAFETY: as portas dos dois 8259 são só desta função, no boot.
    let master = unsafe { PortBus::new(PIC_MASTER) };
    // SAFETY: idem.
    let slave = unsafe { PortBus::new(PIC_SLAVE) };
    pic::mask_all(&master, &slave);
}

fn enable_local_apic(
    mode: ApicMode,
    memory: &mut Memory,
    mapper: &mut X86PageMapper,
) -> Result<(ApicAccess, Option<rustos_hal::VirtAddr>), PlatformError> {
    match mode {
        // SAFETY: o CPUID anunciou x2APIC, e este é o boot em ring 0.
        ApicMode::X2Apic => Ok((unsafe { ApicAccess::enable_x2apic() }, None)),
        ApicMode::XApic { phys } => {
            let virt = map_device(mapper, &mut memory.frames, phys, PAGE_SIZE)
                .map_err(PlatformError::Map)?;
            // SAFETY: o CPUID anunciou APIC e `virt` é a página de `IA32_APIC_BASE`.
            let apic = unsafe { ApicAccess::enable_xapic(virt) };
            Ok((apic, Some(virt)))
        }
    }
}

/// Liga o controlador PS/2; sem ele, o pino do teclado fica mascarado.
fn start_keyboard() -> bool {
    // SAFETY: as portas 0x60 e 0x64 são só do teclado; este é o boot, com IF desligado.
    let mut controller = I8042::new(unsafe { PortBus::new(I8042_PORTS) });
    let ready = controller.init().is_ok();
    if ready {
        *KEYBOARD.lock() = Some(controller);
    }
    ready
}

fn route_io_apic(
    memory: &mut Memory,
    mapper: &mut X86PageMapper,
    destination: u8,
    keyboard: bool,
) -> Result<(), PlatformError> {
    let virt = map_device(
        mapper,
        &mut memory.frames,
        PhysAddr::new(IOAPIC_BASE),
        PAGE_SIZE,
    )
    .map_err(PlatformError::Map)?;
    // SAFETY: `virt` é a página do I/O APIC, mapeada como dispositivo.
    let ioapic = IoApic::new(unsafe { MmioBus::new(virt) });
    let missing = |_| PlatformError::MissingFeature("ioapic pin");
    ioapic
        .route(IOAPIC_PIN_SERIAL, VECTOR_SERIAL, destination, false)
        .map_err(missing)?;
    ioapic
        .route(IOAPIC_PIN_KEYBOARD, VECTOR_KEYBOARD, destination, !keyboard)
        .map_err(missing)
}

/// Mede o TSC e o contador do timer do APIC contra o PIT, e arma o tique.
fn start_clocks(apic: &ApicAccess) -> Result<(), PlatformError> {
    let local = apic.local();
    // SAFETY: as portas do PIT são só desta função, no boot.
    let pit = Pit::new(unsafe { PortBus::new(PIT_PORTS) });
    local.start_countdown();
    let measured = pit::measure(&pit, PIT_WINDOW_TICKS, PIT_MAX_POLLS, || Reading {
        tsc: rdtsc(),
        apic_ticks: local.elapsed_ticks(),
    })
    .ok_or(PlatformError::MissingFeature("pit"))?;
    TSC_HZ.store(measured.tsc_hz, Ordering::Relaxed);
    TSC_ORIGIN.store(rdtsc(), Ordering::Relaxed);
    local.start_periodic(pit::periodic_count(measured.apic_hz, TIMER_HZ));
    Ok(())
}

/// Traduz o número da IRQ para a fonte que o núcleo entende.
pub fn irq_source(irq: Irq) -> IrqSource {
    match u8::try_from(irq.0) {
        Ok(VECTOR_TIMER) => IrqSource::Timer,
        Ok(VECTOR_SERIAL) => IrqSource::Serial,
        Ok(VECTOR_KEYBOARD) => IrqSource::Keyboard,
        Ok(VECTOR_SPURIOUS) => IrqSource::Spurious,
        _ => IrqSource::Unknown,
    }
}

/// O timer do APIC é periódico: não há o que rearmar, e o EOI é do `arch/`.
pub fn timer_tick() {}

/// Ler o `DATA` até esvaziar a FIFO já baixou a linha da UART; o EOI é do `arch/`.
pub fn serial_irq_done() {}

/// Se o controlador PS/2 respondeu no boot.
pub fn has_keyboard() -> bool {
    KEYBOARD.lock().is_some()
}

/// Próxima tecla do PS/2, dentro da IRQ 1; ler o dado baixa a linha.
pub fn keyboard_event() -> Option<KeyEvent> {
    KEYBOARD.lock().as_mut().and_then(I8042::read_event)
}

/// Nanossegundos desde que `init_interrupts` ligou o relógio; `None` antes.
pub fn now_ns() -> Option<u64> {
    let hz = TSC_HZ.load(Ordering::Relaxed);
    (hz != 0).then(|| elapsed_ns(rdtsc(), TSC_ORIGIN.load(Ordering::Relaxed), hz))
}

/// As proteções de hardware desta ISA e se ficaram ligadas.
pub fn protections() -> &'static [Protection] {
    // SAFETY: a escrita foi no boot; depois disso o relatório só é lido.
    unsafe { &*REPORT.0.get() }
}

/// Provoca a exceção pedida; `PageFault` não volta.
pub fn trigger(test: FaultTest) -> bool {
    if !READY.load(Ordering::Acquire) {
        return false;
    }
    match test {
        FaultTest::Breakpoint => {
            // SAFETY: `READY` só vale depois de `install_traps`.
            unsafe { breakpoint() };
        }
        FaultTest::PageFault => {
            let guard = ptr::with_exposed_provenance::<u8>(
                usize::try_from(GUARD_PAGE.load(Ordering::Relaxed)).unwrap_or(0),
            );
            // SAFETY: a página de guarda nunca é mapeada; a leitura só provoca o #PF.
            unsafe { guard.read_volatile() };
        }
    }
    true
}
