//! QEMU `virt` aarch64: PL011 e GICv3 por MMIO, mapeados como dispositivo pelo
//! kernel; exceções e timer pelos registradores de sistema.

use core::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};

use rustos_arch_aarch64::descriptor::Aarch64Format;
pub use rustos_arch_aarch64::fpu::{
    AREA_ALIGN as FPU_AREA_ALIGN, restore as fpu_restore, save as fpu_save,
};
#[cfg(feature = "selftest")]
pub use rustos_arch_aarch64::fpu::{read_probe as fpu_read_probe, write_probe as fpu_write_probe};
use rustos_arch_aarch64::intid::{QEMU_VIRT_PL011_INTID, VIRTUAL_TIMER_INTID};
use rustos_arch_aarch64::sgi::{SGI_RESCHEDULE, SGI_STOP};
#[cfg(feature = "selftest")]
pub use rustos_arch_aarch64::user_blob;
pub use rustos_arch_aarch64::zero::zero_aligned;
use rustos_arch_aarch64::{
    Aarch64Cpu, Aarch64CpuLocal, Aarch64Entropy, Aarch64Ipi, Aarch64Mmu, DirectMapTables, MmioBus,
    QEMU_VIRT_GICD, QEMU_VIRT_GICR, QEMU_VIRT_PL011, active_root, early, exception, gic_cpu, intid,
    prepare_space, protection, timer,
};
pub use rustos_arch_aarch64::{Context, CpuPrefix, call_on_stack, switch};
use rustos_driver_gicv3::{GicError, GicV3, affinity_from_mpidr, find_redistributor, router_for};
use rustos_driver_pl011::Pl011;
use rustos_hal::{
    BootInfo, FrameSource, Handlers, Irq, KeyEvent, MapError, MmuControl, PAGE_SIZE, PageMapper,
    PhysAddr, VirtAddr,
};
use rustos_mm::{PageTables, UserRange};

use super::{FpuSummary, PlatformError, Protection};
use crate::interrupts::{FaultTest, IrqSource};
use crate::memory::{Memory, map_device};

pub const ARCH: &str = "aarch64";

pub type PlatformConsole = Pl011<MmioBus>;
/// As tabelas do mapa da serial precoce: duas páginas que o `kmain` mantém vivas até `early_serial_switch`.
pub type EarlyScratch = early::EarlyTables;
pub type PlatformEntropy = Aarch64Entropy;
/// Espaço de endereçamento: as tabelas da metade alta atrás do `TTBR1_EL1`.
pub type PlatformSpace = PageTables<Aarch64Format, DirectMapTables, Aarch64Mmu>;

/// As tabelas que o `TTBR1_EL1` aponta agora.
///
/// # Safety
/// O mapa direto cobre as tabelas atuais e todo frame novo, e só o chamador mexe nelas.
pub unsafe fn active_space(direct_map_offset: u64) -> PlatformSpace {
    // SAFETY: o índice 7 do MAIR não é usado pelo Limine nem por mapeamento algum ainda.
    unsafe { prepare_space() };
    // SAFETY: EL1 com MMU ligada.
    let root = unsafe { active_root() };
    // SAFETY: o contrato desta função.
    let tables = unsafe { DirectMapTables::new(direct_map_offset) };
    PageTables::new(root, tables, Aarch64Mmu)
}

/// Espaço novo e vazio, para montar antes de ativar.
///
/// # Safety
/// Todo frame de `frames` é acessível em `físico + offset` antes e depois da troca.
pub unsafe fn new_space(
    frames: &mut impl FrameSource,
    direct_map_offset: u64,
) -> Result<PlatformSpace, MapError> {
    // SAFETY: o índice 7 do MAIR não é usado pelo Limine nem por mapeamento algum ainda.
    unsafe { prepare_space() };
    // SAFETY: o contrato desta função.
    let tables = unsafe { DirectMapTables::new(direct_map_offset) };
    PageTables::new_empty(frames, tables, Aarch64Mmu)
}

/// Frequência do tique do timer.
pub const TIMER_HZ: u32 = 100;

/// Pilha das exceções: 4 páginas e a de guarda embaixo.
const EXCEPTION_STACK_PAGES: u64 = 4;
/// Registradores do distribuidor (64 KiB) e de cada redistribuidor (`RD_base` + `SGI_base`).
const GICD_LEN: u64 = 0x1_0000;
const GICR_FRAME_LEN: u64 = 0x2_0000;
/// Prioridade do GIC: quanto menor o número, mais urgente.
const TIMER_PRIORITY: u8 = 0x40;
const SERIAL_PRIORITY: u8 = 0x80;

static UART_BASE: AtomicU64 = AtomicU64::new(0);
/// Base da PL011 que o pânico usa antes do console; zero até haver mapa para ela.
static EARLY_BASE: AtomicU64 = AtomicU64::new(0);
static IDENTITY_MAP: AtomicBool = AtomicBool::new(false);
/// A janela do distribuidor e a da região dos redistribuidores, para cada CPU achar o seu.
static GICD_VIRT: AtomicU64 = AtomicU64::new(0);
static GICR_VIRT: AtomicU64 = AtomicU64::new(0);
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

/// Vetores mínimos e a PL011 por `TTBR0_EL1`, na entrada do `kmain` (SDD 04, DEBT-0002).
pub fn early_init(handlers: &'static Handlers, scratch: &mut EarlyScratch) {
    // SAFETY: EL1 com IRQ mascarada, na entrada do `kmain`.
    unsafe { exception::install_early(handlers) };
    // SAFETY: entrada do `kmain`, antes de `Vm::activate`; o Limine deixa o `TTBR0_EL1` livre.
    if unsafe { early::identity_map_device(scratch, QEMU_VIRT_PL011) } {
        EARLY_BASE.store(QEMU_VIRT_PL011, Ordering::Relaxed);
        IDENTITY_MAP.store(true, Ordering::Relaxed);
    }
}

/// A PL011 como o firmware a deixou: sem programar, sem lock. `None` se não há mapa para ela.
pub fn early_console() -> Option<PlatformConsole> {
    let base = EARLY_BASE.load(Ordering::Relaxed);
    if base == 0 {
        return None;
    }
    // SAFETY: a base é o mapa de 1:1 do `TTBR0_EL1` ou a janela de dispositivos, os dois
    // como dispositivo e com a página inteira da PL011.
    Some(Pl011::new(unsafe { MmioBus::new(VirtAddr::new(base)) }))
}

/// Mapeia a PL011 no espaço novo, antes de ativá-lo: o `TTBR0_EL1` morre com `Vm::activate`.
pub fn early_serial_map(
    space: &mut impl PageMapper,
    frames: &mut impl FrameSource,
) -> Result<(), MapError> {
    let virt = map_device(space, frames, PhysAddr::new(QEMU_VIRT_PL011), PAGE_SIZE)?;
    UART_BASE.store(virt.as_u64(), Ordering::Relaxed);
    Ok(())
}

/// Depois de `Vm::activate`: a serial precoce passa para a janela e o `TTBR0_EL1` volta ao do Limine.
pub fn early_serial_switch(scratch: &mut EarlyScratch) {
    EARLY_BASE.store(UART_BASE.load(Ordering::Relaxed), Ordering::Relaxed);
    if IDENTITY_MAP.swap(false, Ordering::Relaxed) {
        // SAFETY: o mapa de 1:1 foi montado em `early_init` e a serial já usa a janela.
        unsafe { early::release_identity_map(scratch) };
    }
}

pub fn console(
    boot: &impl BootInfo,
    frames: &mut impl FrameSource,
) -> Result<PlatformConsole, MapError> {
    let mut virt = VirtAddr::new(UART_BASE.load(Ordering::Relaxed));
    if virt.as_u64() == 0 {
        // SAFETY: MMU ligada; as tabelas do Limine e os frames novos estão no mapa direto.
        let mut mapper = unsafe { active_space(boot.direct_map_offset()) };
        let registers = PhysAddr::new(QEMU_VIRT_PL011);
        virt = map_device(&mut mapper, frames, registers, PAGE_SIZE)?;
        UART_BASE.store(virt.as_u64(), Ordering::Relaxed);
    }
    // SAFETY: a página foi mapeada como dispositivo e cobre todo o bloco da PL011.
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
    let mut mapper = unsafe { active_space(boot.direct_map_offset()) };
    let stack_top = memory
        .map_stack(&mut mapper, EXCEPTION_STACK_PAGES)
        .map_err(map_error)?;
    let guard = stack_top.as_u64() - (EXCEPTION_STACK_PAGES + 1) * PAGE_SIZE;
    STACK_GUARD.store(guard, Ordering::Relaxed);
    // SAFETY: EL1 com IRQ mascarada; o topo é o da pilha recém-mapeada, só das exceções.
    unsafe { exception::install(handlers, stack_top) }
        .map_err(|_| PlatformError::MissingFeature("EL1h (kernel is not on SP_EL1)"))?;

    let frames = u64::try_from(boot.cpus().count().max(1)).unwrap_or(1);
    let distributor = map_gic(&mut mapper, memory, QEMU_VIRT_GICD, GICD_LEN)?;
    let redistributors = map_gic(&mut mapper, memory, QEMU_VIRT_GICR, frames * GICR_FRAME_LEN)?;
    GICD_VIRT.store(distributor.as_u64(), Ordering::Relaxed);
    GICR_VIRT.store(redistributors.as_u64(), Ordering::Relaxed);
    let mut gic = secondary_gic()?;
    let affinity = affinity_from_mpidr(gic_cpu::mpidr());
    gic.init_distributor().map_err(gic_error)?;
    gic.init_redistributor(affinity).map_err(gic_error)?;
    // SAFETY: EL1 e o redistribuidor desta CPU acabou de acordar.
    unsafe { gic_cpu::enable() }
        .map_err(|_| PlatformError::MissingFeature("GICv3 system registers (ICC_SRE_EL1)"))?;
    enable_local_interrupts(&mut gic)?;
    gic.enable_spi(QEMU_VIRT_PL011_INTID, SERIAL_PRIORITY, router_for(affinity))
        .map_err(gic_error)?;
    serial().enable_rx_interrupt();

    // SAFETY: EL1, primeira e única vez nesta CPU.
    unsafe { timer::start() }.map_err(|error| match error {
        timer::TimerError::NoFrequency => PlatformError::MissingFeature("CNTFRQ_EL0"),
    })?;
    Ok(())
}

/// O distribuidor e o redistribuidor desta CPU, achado percorrendo os frames da região até a
/// afinidade dela ou o `Last` (DEBT-0002 para os endereços).
fn secondary_gic() -> Result<GicV3<MmioBus, MmioBus>, PlatformError> {
    let distributor = VirtAddr::new(GICD_VIRT.load(Ordering::Relaxed));
    let region = VirtAddr::new(GICR_VIRT.load(Ordering::Relaxed));
    // SAFETY: `init_interrupts` mapeou a região inteira dos redistribuidores como dispositivo.
    let whole = unsafe { MmioBus::new(region) };
    let affinity = affinity_from_mpidr(gic_cpu::mpidr());
    let offset = find_redistributor(&whole, affinity).map_err(gic_error)?;
    // SAFETY: o frame achado está dentro da região mapeada e cobre `RD_base` e `SGI_base`.
    let frame = unsafe { MmioBus::new(VirtAddr::new(region.as_u64() + offset as u64)) };
    // SAFETY: `init_interrupts` mapeou o distribuidor como dispositivo.
    let distributor = unsafe { MmioBus::new(distributor) };
    Ok(GicV3::new(distributor, frame))
}

/// O timer e os IPIs desta CPU, no redistribuidor dela.
fn enable_local_interrupts(gic: &mut GicV3<MmioBus, MmioBus>) -> Result<(), PlatformError> {
    gic.enable_ppi(VIRTUAL_TIMER_INTID, TIMER_PRIORITY)
        .map_err(gic_error)?;
    gic.enable_sgi(SGI_RESCHEDULE, TIMER_PRIORITY)
        .map_err(gic_error)?;
    gic.enable_sgi(SGI_STOP, TIMER_PRIORITY).map_err(gic_error)
}

/// Como o timer dispara: o comparador absoluto do `CNTV`, de um tiro.
#[cfg(feature = "selftest")]
pub fn timer_mode() -> &'static str {
    "cntv-cval"
}

/// Programa um tiro para o relógio (`now_ns`) chegar a `deadline_ns`; no passado, dispara já.
pub fn arm_timer(deadline_ns: u64) {
    timer::arm_ns(deadline_ns);
}

/// Nanossegundos que `ticks` do contador de ciclos valem.
pub fn cycles_to_ns(ticks: u64) -> u64 {
    timer::counter_to_ns(ticks)
}

/// Libera FP/SIMD em EL1 e EL0; a troca de contexto salva V0–V31, FPSR e FPCR.
#[allow(
    clippy::unnecessary_wraps,
    reason = "mesma assinatura em todas as ISAs"
)]
pub fn fpu_init() -> Result<FpuSummary, PlatformError> {
    // SAFETY: EL1, uma vez, antes de qualquer troca de contexto.
    unsafe { rustos_arch_aarch64::fpu::init() };
    Ok(FpuSummary {
        mode: "fpsimd",
        bytes: rustos_arch_aarch64::fpu::AREA_BYTES,
        components: "v0-v31 fpsr fpcr",
    })
}

/// Bytes de uma área de estado de FP/SIMD.
pub fn fpu_area_bytes() -> usize {
    rustos_arch_aarch64::fpu::AREA_BYTES
}

/// Estado de reset numa área nova: tudo zero (FPCR e FPSR em reset).
pub fn fpu_init_area(area: &mut [u8]) {
    area.fill(0);
}

fn map_gic(
    mapper: &mut PlatformSpace,
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
        SGI_RESCHEDULE => IrqSource::Reschedule,
        SGI_STOP => IrqSource::Stop,
        QEMU_VIRT_PL011_INTID => IrqSource::Serial,
        intid if !intid::needs_end_of_interrupt(intid) => IrqSource::Spurious,
        _ => IrqSource::Unknown,
    }
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

/// O relógio do aarch64 não se calibra: a frequência vem do `CNTFRQ_EL0`.
#[cfg(feature = "selftest")]
pub fn clock_error_ppm() -> Option<u64> {
    None
}

/// Nanossegundos desde que `init_interrupts` ligou o relógio; `None` antes.
pub fn now_ns() -> Option<u64> {
    timer::now_ns()
}

/// `now_ns` do escalonador: sem o `isb` no aarch64 (SDD 15, "Tempo").
pub fn sched_clock_ns() -> Option<u64> {
    timer::sched_clock_ns()
}

/// Contador virtual (`CNTVCT_EL0`): o relógio da semente de reserva.
pub fn cycles() -> u64 {
    timer::counter()
}

/// `RNDRRS` ou `RNDR`, se `ID_AA64ISAR0_EL1.RNDR` anunciar.
pub fn hardware_entropy() -> Option<PlatformEntropy> {
    Aarch64Entropy::detect()
}

/// O clamp do Spectre v1: `index` se `index < size`, senão 0, com a máscara de `arch/`.
#[inline]
pub fn array_index_nospec(index: usize, size: usize) -> usize {
    index & rustos_arch_aarch64::array_index_mask_nospec(index, size)
}

/// `MIDR_EL1` e os registradores de identificação das mitigações.
pub fn mitigation_features() -> rustos_arch_aarch64::mitigation::MitigationFeatures {
    protection::mitigation_features()
}

/// As proteções de hardware desta ISA e se ficaram ligadas: `pan` e `wxn`.
pub fn protections() -> &'static [Protection] {
    const fn pair(pan: bool, wxn: bool) -> [Protection; 2] {
        [
            Protection {
                name: "pan",
                enabled: pan,
            },
            Protection {
                name: "wxn",
                enabled: wxn,
            },
        ]
    }
    const OFF_OFF: [Protection; 2] = pair(false, false);
    const OFF_ON: [Protection; 2] = pair(false, true);
    const ON_OFF: [Protection; 2] = pair(true, false);
    const ON_ON: [Protection; 2] = pair(true, true);
    let wxn = rustos_arch_aarch64::wxn_active();
    match (PAN_STATE.load(Ordering::Relaxed), wxn) {
        (PAN_ON, true) => &ON_ON,
        (PAN_ON, false) => &ON_OFF,
        (PAN_OFF, true) => &OFF_ON,
        (PAN_OFF, false) => &OFF_OFF,
        (_, true) => &OFF_ON[1..],
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

pub fn cpu_local() -> Aarch64CpuLocal {
    // SAFETY: o kernel roda em EL1.
    unsafe { Aarch64CpuLocal::new() }
}

/// O IPI desta CPU.
///
/// # Safety
/// A interface do GIC desta CPU já está ligada: só há outra CPU para receber depois disso.
pub unsafe fn ipi() -> Aarch64Ipi {
    // SAFETY: o contrato desta função.
    unsafe { Aarch64Ipi::new() }
}

/// O identificador que o IPI usa para esta CPU: a afinidade no formato do `GICR_TYPER`.
pub fn local_hw_id() -> u64 {
    u64::from(affinity_from_mpidr(gic_cpu::mpidr()))
}

/// O identificador de uma CPU como o bootloader o dá (o `MPIDR`), no formato do IPI.
pub fn ipi_target(boot_hw_id: u64) -> u64 {
    u64::from(affinity_from_mpidr(boot_hw_id))
}

/// A raiz das tabelas que esta CPU usa agora: a das tabelas do kernel, depois de `Vm::activate`.
pub fn kernel_root() -> PhysAddr {
    // SAFETY: EL1 com MMU ligada.
    unsafe { active_root() }
}

/// O que uma CPU secundária precisa antes de rodar: a pilha das falhas dela.
#[derive(Debug)]
pub struct SecondaryResources {
    fault_stack_top: VirtAddr,
}

/// Aloca o que a CPU secundária usa; roda na CPU de boot, antes de soltá-la.
pub fn prepare_secondary(
    memory: &mut Memory,
    mapper: &mut PlatformSpace,
) -> Result<SecondaryResources, PlatformError> {
    let fault_stack_top = memory
        .map_stack(mapper, EXCEPTION_STACK_PAGES)
        .map_err(map_error)?;
    Ok(SecondaryResources { fault_stack_top })
}

/// A primeira coisa de uma CPU secundária, ainda na pilha do Limine: as proteções de
/// hardware, o atributo de dispositivo do `MAIR` e as tabelas do kernel.
pub fn secondary_early(root: PhysAddr) {
    harden();
    // SAFETY: o índice 7 do MAIR não é usado por mapeamento algum ainda nesta CPU.
    unsafe { prepare_space() };
    Aarch64Mmu.activate(root);
}

/// Vetores, redistribuidor, interface do GIC, timer e FP/SIMD desta CPU secundária; já em EL1h.
#[allow(
    clippy::needless_pass_by_value,
    reason = "o recurso é entregue a esta CPU e não volta"
)]
pub fn secondary_init(resources: SecondaryResources) -> Result<(), PlatformError> {
    // SAFETY: EL1h com IRQ mascarada, depois de `install` na CPU de boot; a pilha é só desta CPU.
    unsafe { exception::install_secondary(resources.fault_stack_top) }
        .map_err(|_| PlatformError::MissingFeature("EL1h (kernel is not on SP_EL1)"))?;
    let mut gic = secondary_gic()?;
    gic.init_redistributor(affinity_from_mpidr(gic_cpu::mpidr()))
        .map_err(gic_error)?;
    // SAFETY: EL1 e o redistribuidor desta CPU acabou de acordar.
    unsafe { gic_cpu::enable() }
        .map_err(|_| PlatformError::MissingFeature("GICv3 system registers (ICC_SRE_EL1)"))?;
    enable_local_interrupts(&mut gic)?;
    // SAFETY: EL1, depois de `timer::start` na CPU de boot.
    unsafe { timer::start_secondary() };
    fpu_init().map(|_| ())
}

/// O que o kernel escreveu em `[start, start + len)` vai ser executado: o aarch64 não mantém o
/// cache de instrução coerente sozinho.
pub fn sync_icache(start: usize, len: usize) {
    // SAFETY: a faixa é um frame do alocador, mapeado e legível pelo mapa direto.
    unsafe { rustos_arch_aarch64::cache::sync_icache(start, len) };
}

/// A raiz vazia do `TTBR0_EL1`: um frame zerado que a tarefa só de kernel usa. Guardada para o
/// núcleo secundário, que a lê depois de `task::init`.
static EMPTY_ROOT: AtomicU64 = AtomicU64::new(0);

/// Aloca a raiz vazia da metade baixa e liga EL0 nesta CPU (a de boot). Uma vez, depois de
/// `Vm::activate`.
pub fn user_init(frames: &mut impl FrameSource) -> Result<(), MapError> {
    let root = frames.allocate_frame().ok_or(MapError::OutOfFrames)?;
    crate::memory::zero_frame(root);
    EMPTY_ROOT.store(root.as_u64(), Ordering::Release);
    user_cpu_init();
    Ok(())
}

/// O que cada CPU faz antes de rodar tarefa de usuário: o `CNTKCTL_EL1`, a raiz vazia no
/// `TTBR0_EL1` e a metade baixa traduzida.
pub fn user_cpu_init() {
    // SAFETY: EL1 com a MMU ligada e a IRQ mascarada, depois de `Vm::activate` nesta CPU; a raiz
    // vazia é um frame zerado que vive para sempre.
    unsafe { rustos_arch_aarch64::init_user(EMPTY_ROOT.load(Ordering::Acquire)) };
}

/// Troca o espaço de endereçamento desta CPU para o da tarefa que vai rodar; `root` zero é
/// tarefa do kernel. Com a IRQ mascarada. A pilha de entrada é a própria `SP_EL1`.
pub fn switch_space(root: u64, _kernel_stack_top: u64) {
    // SAFETY: o chamador mascara a IRQ, depois de `user_cpu_init` nesta CPU; `root` é de um
    // espaço de `new_user_space`.
    unsafe { rustos_arch_aarch64::switch_space(root) };
}

/// Primeira volta a EL0. Nunca volta.
///
/// # Safety
/// Contrato de `rustos_arch_aarch64::enter_user`.
pub unsafe fn enter_user(entry: u64, sp: u64, arg0: u64, arg1: u64, kernel_top: u64) -> ! {
    // SAFETY: o contrato desta função.
    unsafe { rustos_arch_aarch64::enter_user(entry, sp, arg0, arg1, kernel_top) }
}

/// Espaço de usuário novo: uma raiz vazia para o `TTBR0_EL1` (a metade do kernel é o `TTBR1_EL1`).
///
/// # Safety
/// Contrato de `new_space`.
pub unsafe fn new_user_space(
    frames: &mut impl FrameSource,
    direct_map_offset: u64,
) -> Result<PlatformSpace, MapError> {
    // SAFETY: o contrato desta função.
    unsafe { new_space(frames, direct_map_offset) }
}

/// No aarch64 a metade do kernel não está na raiz do usuário: não há o que dessincronizar.
#[allow(
    clippy::unnecessary_wraps,
    clippy::needless_pass_by_ref_mut,
    reason = "mesma assinatura em todas as ISAs"
)]
pub fn kernel_half_in_sync(_space: &PlatformSpace) -> bool {
    true
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
        rustos_arch_aarch64::usercopy::copy_from_user(dst.as_mut_ptr(), range.addr(), dst.len())
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
        rustos_arch_aarch64::usercopy::copy_to_user(range.addr(), src.as_ptr(), src.len())
    };
    missing == 0
}

/// No aarch64 não há `AC`; o `SCTLR.SPAN` zerado religa o PAN a cada exceção.
#[cfg(feature = "selftest")]
pub fn ac_set() -> bool {
    false
}

/// O aarch64 não tem `AC`: não há o que contar.
#[cfg(feature = "selftest")]
pub fn copy_fixup_stats() -> Option<(usize, usize)> {
    None
}

/// O `eret` é a única saída para EL0: não há saída rápida a ligar.
#[cfg(feature = "selftest")]
pub const HAS_FAST_RETURN: bool = false;

/// Sem efeito no aarch64.
#[cfg(feature = "selftest")]
pub fn set_fast_return(_on: bool) {}

/// Liga ou desliga a limpeza da pilha do kernel na volta ao usuário (o `arch/` pula o gancho).
#[cfg(feature = "selftest")]
pub fn set_stack_clean(on: bool) {
    rustos_arch_aarch64::exception::set_scrub(on);
}

/// Liga ou desliga a invalidação só local do `entry_removed` (modo de teste do *shootdown*).
#[cfg(feature = "selftest")]
pub fn set_local_only(on: bool) {
    rustos_arch_aarch64::set_local_only(on);
}
