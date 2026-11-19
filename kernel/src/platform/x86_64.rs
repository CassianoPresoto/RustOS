//! PC x86_64: COM1 por porta de E/S, GDT/IDT próprias, Local APIC, I/O APIC e
//! relógio pelo TSC calibrado contra o PIT.

use alloc::boxed::Box;
use core::cell::UnsafeCell;
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use rustos_arch_x86_64::clock::{absolute_deadline, ns_to_ticks};
pub use rustos_arch_x86_64::fpu::{
    AREA_ALIGN as FPU_AREA_ALIGN, restore as fpu_restore, save as fpu_save,
};
use rustos_arch_x86_64::ioapic::{IOAPIC_BASE, IoApic};
use rustos_arch_x86_64::lapic::{
    VECTOR_KEYBOARD, VECTOR_RESCHEDULE, VECTOR_SERIAL, VECTOR_SHOOTDOWN, VECTOR_SPURIOUS,
    VECTOR_STOP, VECTOR_TIMER,
};
use rustos_arch_x86_64::pic;
use rustos_arch_x86_64::pit::{self, Pit, Reading};
#[cfg(feature = "selftest")]
pub use rustos_arch_x86_64::user_blob;
use rustos_arch_x86_64::{
    ApicAccess, ApicMode, COM1, CpuTables, IstStacks, MmioBus, PIC_MASTER, PIC_SLAVE, PIT_PORTS,
    PortBus, Protections, TimerMode, X86Cpu, X86CpuLocal, X86Entropy, X86Ipi, X86Mmu, active_root,
    arm_oneshot, arm_tsc_deadline, breakpoint, cpu_features, detect_apic, enable_protections,
    enable_secondary_apic, install_early_traps, install_traps, load_cpu_tables, local_apic_id,
    publish_apic, rdtsc, require_features, set_timer_mode, xstate,
};
pub use rustos_arch_x86_64::{Context, CpuPrefix, call_on_stack, switch};
use rustos_arch_x86_64::{DirectMapTables, pte::X86Format};
use rustos_driver_i8042::I8042;
use rustos_driver_uart16550::{DIVISOR_115200, Uart16550};
use rustos_hal::{
    BootInfo, FrameSource, Handlers, Irq, KeyEvent, MapError, MmuControl, PAGE_SIZE, PageMapper,
    PhysAddr, TickScale,
};
use rustos_mm::{PageTables, UserRange};
use rustos_sync::SpinLock;

use super::{FpuSummary, PlatformError, Protection};
use crate::interrupts::{FaultTest, IrqSource};
use crate::memory::{Memory, map_device};

pub const ARCH: &str = "x86_64";

pub type PlatformConsole = Uart16550<PortBus>;
/// No PC a serial precoce não precisa de memória: o tipo existe só para a assinatura ser a mesma.
#[derive(Debug)]
pub struct EarlyScratch;

impl EarlyScratch {
    pub const fn new() -> Self {
        Self
    }
}
pub type PlatformEntropy = X86Entropy;
/// Espaço de endereçamento: as tabelas de 4 níveis atrás do `CR3`.
pub type PlatformSpace = PageTables<X86Format, DirectMapTables, X86Mmu>;

/// As tabelas que o `CR3` aponta agora.
///
/// # Safety
/// O mapa direto cobre as tabelas atuais e todo frame novo, e só o chamador mexe nelas.
pub unsafe fn active_space(direct_map_offset: u64) -> PlatformSpace {
    // SAFETY: ring 0 com paginação de 4 níveis no boot.
    let root = unsafe { active_root() };
    // SAFETY: o contrato desta função.
    let tables = unsafe { DirectMapTables::new(direct_map_offset) };
    PageTables::new(root, tables, X86Mmu)
}

/// Espaço novo e vazio, para montar antes de ativar.
///
/// # Safety
/// Todo frame de `frames` é acessível em `físico + offset` antes e depois da troca.
pub unsafe fn new_space(
    frames: &mut impl FrameSource,
    direct_map_offset: u64,
) -> Result<PlatformSpace, MapError> {
    // SAFETY: ring 0 no boot.
    unsafe { require_features() }?;
    // SAFETY: o contrato desta função.
    let tables = unsafe { DirectMapTables::new(direct_map_offset) };
    PageTables::new_empty(frames, tables, X86Mmu)
}

/// Frequência do tique do timer.
pub const TIMER_HZ: u32 = 100;

/// Páginas de cada pilha de exceção (#DF, NMI, #MC, #DB); a de guarda fica abaixo.
const IST_STACK_PAGES: u64 = 4;
/// Calibração pelo PIT (SDD 10, DEBT-0066): fecha em 400 ppm, aceita o que houver aos 5 ms se
/// mais tempo não chega lá, para em 20 ms, recusa acima de 1% e desiste do PIT parado em 300 M ciclos.
const PIT_POLICY: pit::Policy = pit::Policy {
    min_ticks: 5_966,
    max_ticks: 23_864,
    target_ppm: 400,
    refuse_ppm: 10_000,
    max_cycles: 300_000_000,
};
/// Incerteza acima da qual o boot avisa: 0,5%. O limite é de 3 a 5 vezes o erro visto em TCG.
const PIT_WARN_PPM: u64 = 5_000;
/// Pinos do I/O APIC da QEMU `q35`: as ISA 1 (teclado) e 4 (COM1) vão direto,
/// sem redirecionamento do MADT (DEBT-0009).
const IOAPIC_PIN_KEYBOARD: u8 = 1;
const IOAPIC_PIN_SERIAL: u8 = 4;
/// Base das portas do controlador PS/2: dado em 0x60, status e comando em 0x64.
const I8042_PORTS: u16 = 0x60;

/// Depois do boot, só a IRQ do teclado pega este lock: ninguém o segura quando
/// ela chega. Quem quer saber se há teclado lê `KEYBOARD_PRESENT`.
static KEYBOARD: SpinLock<Option<I8042<PortBus>>> = SpinLock::new(None);
static KEYBOARD_PRESENT: AtomicBool = AtomicBool::new(false);

static READY: AtomicBool = AtomicBool::new(false);
/// Primeira página, sem mapeamento, da pilha de #DF: lê-la provoca um #PF.
static GUARD_PAGE: AtomicU64 = AtomicU64::new(0);
static TSC_HZ: AtomicU64 = AtomicU64::new(0);
/// Limite do erro da calibração do TSC, em ppm; 0 antes dela.
static CLOCK_ERROR_PPM: AtomicU64 = AtomicU64::new(0);
static TSC_ORIGIN: AtomicU64 = AtomicU64::new(0);
/// `TickScale` do TSC, em dois atômicos: `now_ns` não divide.
static TSC_MULT: AtomicU64 = AtomicU64::new(0);
static TSC_SHIFT: AtomicU32 = AtomicU32::new(0);
/// Frequência do contador do timer do APIC, já com o divisor.
static APIC_HZ: AtomicU64 = AtomicU64::new(0);
static TSC_DEADLINE: AtomicBool = AtomicBool::new(false);

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

/// Liga as proteções de hardware da CPU. Primeira coisa depois da memória: o NX tem
/// de estar no `EFER` antes de as tabelas novas usarem o bit.
pub fn harden() {
    // SAFETY: ring 0 no boot, e o kernel não toca memória de usuário.
    publish_protections(unsafe { enable_protections() });
}

/// Zera `[ptr, ptr + len)` com o `memset` do alvo, que já é `rep stosq` (DEBT-0058).
///
/// # Safety
/// `ptr` e `len` múltiplos de 64; a faixa é memória gravável do kernel, sem outro acesso.
pub unsafe fn zero_aligned(ptr: *mut u8, len: usize) {
    // SAFETY: o contrato desta função.
    unsafe { ptr.write_bytes(0, len) };
}

pub fn cpu() -> X86Cpu {
    // SAFETY: o Limine entrega o kernel em ring 0, e ele nunca sai de lá.
    unsafe { X86Cpu::new() }
}

/// Vetores mínimos sobre a GDT do Limine: exceção do começo do boot sai pela serial.
pub fn early_init(handlers: &'static Handlers, _scratch: &mut EarlyScratch) {
    // SAFETY: ring 0 com IF zerado, na entrada do `kmain`; a GDT do Limine está carregada.
    unsafe { install_early_traps(handlers) };
}

/// A COM1 como o firmware a deixou: sem programar, sem lock.
#[allow(
    clippy::unnecessary_wraps,
    reason = "mesma assinatura em todas as ISAs"
)]
pub fn early_console() -> Option<PlatformConsole> {
    // SAFETY: só o console e o pânico usam a COM1, e o pânico para os outros antes de escrever.
    Some(Uart16550::new(unsafe { PortBus::new(COM1) }))
}

/// No PC a COM1 é porta de E/S: não há o que mapear.
#[allow(
    clippy::unnecessary_wraps,
    reason = "mesma assinatura em todas as ISAs"
)]
pub fn early_serial_map(
    _space: &mut impl PageMapper,
    _frames: &mut impl FrameSource,
) -> Result<(), MapError> {
    Ok(())
}

/// Nada a trocar no PC.
pub fn early_serial_switch(_scratch: &mut EarlyScratch) {}

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
    let features = cpu_features();
    if !features.tsc {
        return Err(PlatformError::MissingFeature("tsc"));
    }
    let mode = detect_apic(features).ok_or(PlatformError::MissingFeature("apic"))?;
    // SAFETY: o mapa direto cobre as tabelas e os frames; só o boot mexe nelas agora.
    let mut mapper = unsafe { active_space(boot.direct_map_offset()) };

    let stacks = IstStacks {
        double_fault: ist_stack(memory, &mut mapper)?,
        nmi: ist_stack(memory, &mut mapper)?,
        machine_check: ist_stack(memory, &mut mapper)?,
        debug: ist_stack(memory, &mut mapper)?,
    };
    let guard = stacks.double_fault.as_u64() - (IST_STACK_PAGES + 1) * PAGE_SIZE;
    GUARD_PAGE.store(guard, Ordering::Relaxed);
    let tables = Box::leak(Box::new(CpuTables::new()));
    // SAFETY: ring 0 no boot com IF desligado; as pilhas vêm de `map_stack`; só esta CPU usa `tables`.
    unsafe { install_traps(stacks, handlers, tables) };
    READY.store(true, Ordering::Release);

    mask_legacy_pic();
    let (apic, mmio) = enable_local_apic(mode, memory, &mut mapper)?;
    apic.local().enable();
    let destination = u8::try_from(apic.local().id(apic.is_x2apic()))
        .map_err(|_| PlatformError::MissingFeature("8-bit apic id"))?;
    let keyboard = start_keyboard();
    route_io_apic(memory, &mut mapper, destination, keyboard)?;
    // Tecla que chegou antes de o pino abrir deixa a linha alta sem nova borda.
    while keyboard_event().is_some() {}
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
    mapper: &mut PlatformSpace,
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
        KEYBOARD_PRESENT.store(true, Ordering::Release);
    }
    ready
}

fn route_io_apic(
    memory: &mut Memory,
    mapper: &mut PlatformSpace,
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

/// Mede o TSC e o contador do timer do APIC contra o PIT e escolhe o modo de
/// um tiro: TSC-deadline se o CPUID anuncia, senão o contador do APIC.
fn start_clocks(apic: &ApicAccess) -> Result<(), PlatformError> {
    let local = apic.local();
    // SAFETY: as portas do PIT são só desta função, no boot.
    let pit = Pit::new(unsafe { PortBus::new(PIT_PORTS) });
    local.start_countdown();
    let measured = pit::calibrate(&pit, PIT_POLICY, || Reading {
        tsc: rdtsc(),
        apic_ticks: local.elapsed_ticks(),
    })
    .map_err(|failure| match failure {
        pit::Failure::Stuck => PlatformError::MissingFeature("pit: the counter does not move"),
        pit::Failure::TooUncertain(_) => {
            PlatformError::MissingFeature("pit: calibration error above 1%")
        }
    })?;
    CLOCK_ERROR_PPM.store(measured.error_ppm, Ordering::Relaxed);
    if measured.error_ppm > PIT_WARN_PPM {
        kprintln!(
            "[rustos] tsc: calibrated to within {} ppm (target {PIT_WARN_PPM})",
            measured.error_ppm
        );
    }
    TSC_HZ.store(measured.tsc_hz, Ordering::Relaxed);
    let (mult, shift) = TickScale::new(measured.tsc_hz).map_or((0, 0), TickScale::parts);
    TSC_ORIGIN.store(rdtsc(), Ordering::Relaxed);
    TSC_SHIFT.store(shift, Ordering::Relaxed);
    // O `mult` publica o relógio: gravado por último, quem o vê já vê origem e `shift`.
    TSC_MULT.store(mult, Ordering::Release);
    APIC_HZ.store(measured.apic_hz, Ordering::Relaxed);
    let deadline = cpu_features().tsc_deadline;
    TSC_DEADLINE.store(deadline, Ordering::Relaxed);
    set_timer_mode(if deadline {
        TimerMode::TscDeadline
    } else {
        TimerMode::OneShot
    });
    Ok(())
}

/// Limite do erro da calibração do relógio contra o PIT, em ppm.
#[allow(
    clippy::unnecessary_wraps,
    reason = "mesma assinatura em todas as ISAs"
)]
#[cfg(feature = "selftest")]
pub fn clock_error_ppm() -> Option<u64> {
    Some(CLOCK_ERROR_PPM.load(Ordering::Relaxed))
}

/// Como o timer dispara: `tsc-deadline` ou `one-shot` (contador do APIC).
#[cfg(feature = "selftest")]
pub fn timer_mode() -> &'static str {
    if TSC_DEADLINE.load(Ordering::Relaxed) {
        "tsc-deadline"
    } else {
        "one-shot"
    }
}

/// Programa um tiro para o relógio (`now_ns`) chegar a `deadline_ns`; no passado,
/// dispara já. O caminho TSC-deadline nunca rodou (DEBT-0034).
pub fn arm_timer(deadline_ns: u64) {
    if TSC_DEADLINE.load(Ordering::Relaxed) {
        let ticks = ns_to_ticks(deadline_ns, TSC_HZ.load(Ordering::Relaxed));
        // SAFETY: `TSC_DEADLINE` só vale com o CPUID anunciando o recurso e o modo ligado.
        unsafe { arm_tsc_deadline(absolute_deadline(TSC_ORIGIN.load(Ordering::Relaxed), ticks)) };
        return;
    }
    let delta = deadline_ns.saturating_sub(now_ns().unwrap_or(0));
    let count = ns_to_ticks(delta, APIC_HZ.load(Ordering::Relaxed));
    arm_oneshot(u32::try_from(count).unwrap_or(u32::MAX));
}

/// Nanossegundos que `ticks` do contador de ciclos valem.
pub fn cycles_to_ns(ticks: u64) -> u64 {
    tsc_scale().convert(ticks)
}

/// Liga x87, SSE e XSAVE; o relatório diz o que a troca de contexto salva.
pub fn fpu_init() -> Result<FpuSummary, PlatformError> {
    // SAFETY: ring 0, uma vez, antes de qualquer troca de contexto.
    let info = unsafe { rustos_arch_x86_64::fpu::init(cpu_features()) }
        .map_err(|_| PlatformError::MissingFeature("sse"))?;
    Ok(FpuSummary {
        mode: info.mode.name(),
        bytes: info.bytes,
        components: xstate::components(info.xcr0),
    })
}

/// Bytes de uma área de estado de FPU/SIMD.
pub fn fpu_area_bytes() -> usize {
    rustos_arch_x86_64::fpu::area_bytes()
}

/// Estado de reset numa área nova.
pub fn fpu_init_area(area: &mut [u8]) {
    xstate::init_area(area);
}

#[cfg(feature = "selftest")]
pub use rustos_arch_x86_64::fpu::{read_probe as fpu_read_probe, write_probe as fpu_write_probe};

/// Traduz o número da IRQ para a fonte que o núcleo entende.
pub fn irq_source(irq: Irq) -> IrqSource {
    match u8::try_from(irq.0) {
        Ok(VECTOR_TIMER) => IrqSource::Timer,
        Ok(VECTOR_SERIAL) => IrqSource::Serial,
        Ok(VECTOR_KEYBOARD) => IrqSource::Keyboard,
        Ok(VECTOR_RESCHEDULE) => IrqSource::Reschedule,
        Ok(VECTOR_STOP) => IrqSource::Stop,
        Ok(VECTOR_SHOOTDOWN) => IrqSource::Shootdown,
        Ok(VECTOR_SPURIOUS) => IrqSource::Spurious,
        _ => IrqSource::Unknown,
    }
}

/// Ler o `DATA` até esvaziar a FIFO já baixou a linha da UART; o EOI é do `arch/`.
pub fn serial_irq_done() {}

/// Se o controlador PS/2 respondeu no boot.
pub fn has_keyboard() -> bool {
    KEYBOARD_PRESENT.load(Ordering::Acquire)
}

/// Próxima tecla do PS/2, dentro da IRQ 1; ler o dado baixa a linha.
pub fn keyboard_event() -> Option<KeyEvent> {
    KEYBOARD.lock().as_mut().and_then(I8042::read_event)
}

/// Nanossegundos desde que `init_interrupts` ligou o relógio; `None` antes.
pub fn now_ns() -> Option<u64> {
    let scale = tsc_scale();
    (!scale.is_zero())
        .then(|| scale.convert(rdtsc().saturating_sub(TSC_ORIGIN.load(Ordering::Relaxed))))
}

/// Lê o `mult` primeiro (acquire): quem o vê diferente de zero vê o `shift` já gravado.
fn tsc_scale() -> TickScale {
    let mult = TSC_MULT.load(Ordering::Acquire);
    TickScale::from_parts(mult, TSC_SHIFT.load(Ordering::Relaxed))
}

/// `now_ns` do escalonador: o `rdtsc` já não é ordenado, como no `sched_clock` do Linux.
pub fn sched_clock_ns() -> Option<u64> {
    now_ns()
}

/// Contador de ciclos (TSC): o relógio da semente de reserva.
pub fn cycles() -> u64 {
    rdtsc()
}

/// `RDSEED` ou `RDRAND`, se o CPUID anunciar.
pub fn hardware_entropy() -> Option<PlatformEntropy> {
    X86Entropy::detect(cpu_features())
}

/// O clamp do Spectre v1: `index` se `index < size`, senão 0, com a máscara de `arch/`.
#[inline]
pub fn array_index_nospec(index: usize, size: usize) -> usize {
    index & rustos_arch_x86_64::array_index_mask_nospec(index, size)
}

/// O que o CPUID e o `IA32_ARCH_CAPABILITIES` dizem das mitigações.
pub fn mitigation_features() -> rustos_arch_x86_64::mitigation::MitigationFeatures {
    // SAFETY: ring 0; o `rdmsr` só roda se o CPUID anuncia o MSR.
    unsafe { rustos_arch_x86_64::mitigation_features() }
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
        FaultTest::WriteText => crate::vm::probe_write_text(),
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

#[allow(
    clippy::inline_always,
    reason = "no caminho da syscall nula: um quadro a mais passa da janela que o `arch/` repinta"
)]
#[inline(always)]
pub fn cpu_local() -> X86CpuLocal {
    // SAFETY: o kernel roda em ring 0.
    unsafe { X86CpuLocal::new() }
}

/// O IPI desta CPU.
///
/// # Safety
/// O Local APIC desta CPU já foi publicado: só há outra CPU para receber depois disso.
pub unsafe fn ipi() -> X86Ipi {
    // SAFETY: o contrato desta função.
    unsafe { X86Ipi::new() }
}

/// Manda o IPI de invalidação do TLB à CPU `hw_id`.
pub fn send_shootdown(hw_id: u64) {
    // SAFETY: só há CPU de destino com outras CPUs online, e o APIC desta já foi publicado.
    unsafe { ipi() }.send_shootdown(hw_id);
}

/// Invalida `pages` páginas a partir de `start` no TLB desta CPU.
pub fn flush_pages(start: u64, pages: u64) {
    // SAFETY: ring 0, com a IRQ mascarada ou dentro dela.
    unsafe { rustos_arch_x86_64::flush_pages(start, pages) };
}

/// O identificador que o IPI usa para esta CPU: o LAPIC ID.
pub fn local_hw_id() -> u64 {
    local_apic_id().map_or(0, u64::from)
}

/// O identificador de uma CPU como o bootloader o dá, no formato do IPI: o LAPIC ID já é o dele.
pub const fn ipi_target(boot_hw_id: u64) -> u64 {
    boot_hw_id
}

/// A raiz das tabelas que esta CPU usa agora: a das tabelas do kernel, depois de `Vm::activate`.
pub fn kernel_root() -> PhysAddr {
    // SAFETY: ring 0 com paginação de 4 níveis.
    unsafe { active_root() }
}

/// O que uma CPU secundária precisa antes de rodar: pilhas IST, GDT e TSS só dela.
#[derive(Debug)]
pub struct SecondaryResources {
    ist: IstStacks,
    tables: &'static mut CpuTables,
}

/// Aloca o que a CPU secundária usa; roda na CPU de boot, antes de soltá-la.
pub fn prepare_secondary(
    memory: &mut Memory,
    mapper: &mut PlatformSpace,
) -> Result<SecondaryResources, PlatformError> {
    let ist = IstStacks {
        double_fault: ist_stack(memory, mapper)?,
        nmi: ist_stack(memory, mapper)?,
        machine_check: ist_stack(memory, mapper)?,
        debug: ist_stack(memory, mapper)?,
    };
    Ok(SecondaryResources {
        ist,
        tables: Box::leak(Box::new(CpuTables::new())),
    })
}

/// A primeira coisa de uma CPU secundária, ainda na pilha do Limine: as proteções de
/// hardware e as tabelas do kernel, na mesma ordem da CPU de boot.
pub fn secondary_early(root: PhysAddr) {
    // SAFETY: ring 0; o kernel não lê nem executa memória de usuário.
    let _ = unsafe { enable_protections() };
    X86Mmu.activate(root);
}

/// GDT, TSS e IDT, Local APIC, timer e FPU/SIMD desta CPU secundária.
#[allow(
    clippy::needless_pass_by_value,
    reason = "o recurso é entregue a esta CPU e não volta"
)]
pub fn secondary_init(resources: SecondaryResources) -> Result<(), PlatformError> {
    // SAFETY: ring 0 com IF desligado, depois de `install_traps` na CPU de boot; as pilhas e as
    // tabelas são só desta CPU.
    unsafe { load_cpu_tables(resources.ist, resources.tables) };
    // SAFETY: ring 0, depois de `publish_apic` na CPU de boot, que escolheu o modo pelo CPUID.
    if !unsafe { enable_secondary_apic() } {
        return Err(PlatformError::MissingFeature("apic"));
    }
    set_timer_mode(if TSC_DEADLINE.load(Ordering::Relaxed) {
        TimerMode::TscDeadline
    } else {
        TimerMode::OneShot
    });
    fpu_init().map(|_| ())
}

/// O x86_64 mantém o cache de instrução coerente com as escritas: nada a fazer.
pub fn sync_icache(_start: usize, _len: usize) {}

/// A raiz do kernel e o que o usuário precisa antes da primeira tarefa. Uma vez, depois de
/// `Vm::activate`; o x86_64 não aloca nada.
#[allow(
    clippy::unnecessary_wraps,
    reason = "mesma assinatura em todas as ISAs"
)]
pub fn user_init(_frames: &mut impl FrameSource) -> Result<(), MapError> {
    rustos_arch_x86_64::set_kernel_root(kernel_root().as_u64());
    Ok(())
}

/// O que cada CPU faz antes de rodar tarefa de usuário: no x86_64, nada (`load_cpu` já ligou o
/// `SYSCALL`).
pub fn user_cpu_init() {}

/// Troca o espaço de endereçamento desta CPU para o da tarefa que vai rodar; `root` zero é
/// tarefa do kernel. Com a IRQ mascarada.
pub fn switch_space(root: u64, kernel_stack_top: u64) {
    // SAFETY: o chamador mascara a IRQ, `root` é de um espaço montado por `new_user_space` (com a
    // metade do kernel) e o topo é o da pilha da tarefa.
    unsafe { rustos_arch_x86_64::switch_space(root, kernel_stack_top) };
}

/// Primeira volta ao anel 3. Nunca volta.
///
/// # Safety
/// Contrato de `rustos_arch_x86_64::enter_user`.
pub unsafe fn enter_user(entry: u64, sp: u64, arg0: u64, arg1: u64, _kernel_top: u64) -> ! {
    // SAFETY: o contrato desta função.
    unsafe { rustos_arch_x86_64::enter_user(entry, sp, arg0, arg1) }
}

/// Espaço de usuário novo: raiz própria com a metade do kernel copiada por ponteiro.
///
/// # Safety
/// Contrato de `new_space`, e a CPU já está nas tabelas do kernel (`kernel_root`).
pub unsafe fn new_user_space(
    frames: &mut impl FrameSource,
    direct_map_offset: u64,
) -> Result<PlatformSpace, MapError> {
    // SAFETY: o contrato desta função.
    let mut space = unsafe { new_space(frames, direct_map_offset) }?;
    space.adopt_top_entries(kernel_root(), KERNEL_TOP_ENTRIES);
    Ok(space)
}

/// As entradas de nível alto do kernel: a metade de cima da raiz.
const KERNEL_TOP_ENTRIES: core::ops::Range<usize> = 256..512;

/// A raiz do espaço tem a mesma metade do kernel que a raiz do kernel. Entrada nova do kernel
/// depois da cópia dessincronizaria as raízes.
pub fn kernel_half_in_sync(space: &PlatformSpace) -> bool {
    space.top_entries_match(kernel_root(), KERNEL_TOP_ENTRIES)
}

/// Copia `dst.len()` bytes de `range` (usuário) para `dst`. `false` se uma página faltou.
pub fn copy_from_user(range: UserRange, dst: &mut [u8]) -> bool {
    assert_eq!(
        range.len(),
        dst.len() as u64,
        "copy_from_user: tamanhos diferentes"
    );
    // SAFETY: `dst` é memória do kernel do tamanho da faixa e a faixa é de usuário (`UserRange`).
    let missing = unsafe {
        rustos_arch_x86_64::usercopy::copy_from_user(dst.as_mut_ptr(), range.addr(), dst.len())
    };
    missing == 0
}

/// Copia `src` para `range` (usuário). `false` se uma página faltou.
pub fn copy_to_user(range: UserRange, src: &[u8]) -> bool {
    assert_eq!(
        range.len(),
        src.len() as u64,
        "copy_to_user: tamanhos diferentes"
    );
    // SAFETY: `src` é memória do kernel do tamanho da faixa e a faixa é de usuário (`UserRange`).
    let missing = unsafe {
        rustos_arch_x86_64::usercopy::copy_to_user(range.addr(), src.as_ptr(), src.len())
    };
    missing == 0
}

/// O kernel roda com `EFLAGS.AC` ligado, desarmando o SMAP? Sem SMAP na CPU a entrada de exceção
/// não faz `clac` e o `AC` no anel 0 não tem efeito: não conta.
#[cfg(feature = "selftest")]
pub fn ac_set() -> bool {
    let has_smap = protections()
        .iter()
        .any(|protection| protection.name == "smap" && protection.enabled);
    has_smap && rustos_arch_x86_64::ac_flag_set()
}

/// Faltas de cópia recuperadas e quantas rodaram o tratador com `AC` ligado.
#[cfg(feature = "selftest")]
#[allow(
    clippy::unnecessary_wraps,
    reason = "mesma assinatura em todas as ISAs"
)]
pub fn copy_fixup_stats() -> Option<(usize, usize)> {
    Some(rustos_arch_x86_64::usercopy::fixup_stats())
}

/// Esta ISA tem saída rápida de syscall (`SYSRET`) que se liga e desliga.
#[cfg(feature = "selftest")]
pub const HAS_FAST_RETURN: bool = true;

/// Liga ou desliga a saída por `SYSRET` (o `IRETQ` é sempre o caminho de reserva).
#[cfg(feature = "selftest")]
pub fn set_fast_return(on: bool) {
    rustos_arch_x86_64::set_sysret(on);
}

/// Liga ou desliga a limpeza da pilha do kernel na volta ao usuário (o `arch/` pula o gancho).
#[cfg(feature = "selftest")]
pub fn set_stack_clean(on: bool) {
    rustos_arch_x86_64::set_scrub(on);
}
