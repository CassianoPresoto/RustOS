//! Raiz de composição do RustOS: o único crate que conhece todos os adaptadores.
//! Aqui se decide qual implementação atende cada porta; política mora em `core/`.

#![no_std]
#![no_main]

extern crate alloc;

#[cfg(not(target_os = "none"))]
compile_error!("o kernel só compila para alvos bare-metal: use `cargo xtask build`");

mod addr;
#[cfg(feature = "selftest")]
mod bench;
mod boot;
mod boot_time;
#[macro_use]
mod console;
mod early;
mod fmt;
mod frames;
mod heap;
mod interrupts;
mod memory;
mod panic;
mod percpu;
mod platform;
mod random;
mod screen;
#[cfg(feature = "selftest")]
mod selftest;
#[cfg(not(feature = "selftest"))]
#[path = "selftest_off.rs"]
mod selftest;
mod smp;
mod stack;
mod sync;
mod system;
mod task;
mod vm;

use alloc::boxed::Box;

use rustos_boot_limine::LimineBoot;
use rustos_hal::BootInfo;
use rustos_hal::redact;
use rustos_shell::DisplayInfo;

use crate::boot::BootRecord;

/// O `cargo xtask smoke` espera esta linha na serial; mudar aqui quebra o CI.
pub const BOOT_OK_MARKER: &str = "[rustos] boot ok";

/// Páginas da pilha do kernel: 64 KiB, o mínimo que o Limine garante e com que o
/// kernel já rodava, mais uma página de guarda abaixo.
const KERNEL_STACK_PAGES: u64 = 16;

/// O que passa da pilha do bootloader para a do kernel.
struct Handoff {
    boot: BootRecord,
    memory: memory::Memory,
    vm: vm::Vm,
    entry_cycles: u64,
    /// `rustos.selftest=1`: autotestes e benchmarks completos no boot (SDD 04); a imagem enxuta o recusa.
    selftest: bool,
}

#[unsafe(no_mangle)]
extern "C" fn kmain() -> ! {
    // Fica nesta pilha até `early_serial_switch`: a MMU lê as tabelas dele (aarch64).
    let mut early_scratch = platform::EarlyScratch::new();
    early::init(&mut early_scratch);
    let entry_cycles = platform::cycles();
    boot_time::start(entry_cycles);
    percpu::install(0);
    percpu::mark_boot_online();
    // SAFETY: primeira coisa na entrada; nada reaproveitou a memória do bootloader.
    let limine = match unsafe { LimineBoot::from_bootloader() } {
        Ok(boot) => boot,
        Err(error) => panic!("boot protocol: {error:?}"),
    };
    let params = {
        // SAFETY: a memória das respostas ainda não foi reaproveitada.
        let line = unsafe { limine.command_line() };
        rustos_hal::BootParams::parse(line.unwrap_or(""))
    };
    #[cfg(feature = "panic-tests")]
    {
        early::test_point(params, rustos_hal::EarlyPanic::Entry);
        early::test_point(params, rustos_hal::EarlyPanic::Fault);
    }
    // SAFETY: chamada única; as regiões utilizáveis estão livres e no mapa direto.
    let mut memory = match unsafe { memory::Memory::init(&limine) } {
        Ok(memory) => memory,
        Err(error) => panic!("memory: {error:?}"),
    };
    // Daqui em diante nada lê a resposta do Limine: a memória dela volta ao alocador.
    let boot = BootRecord::capture(&limine);
    platform::harden();
    // SAFETY: o mapa direto do Limine cobre todo frame do alocador e toda região de `boot`.
    let mut vm = match unsafe { vm::Vm::build(&boot, &mut memory.frames) } {
        Ok(vm) => vm,
        Err(error) => panic!("vm: {error:?}"),
    };
    if let Err(error) = memory::Memory::place_windows(&vm.occupied()) {
        panic!("address windows: {error:?}");
    }
    addr::set_direct(vm.occupied()[0]);
    vm.map_initramfs(&boot, &mut memory.frames);
    if let Err(error) = platform::early_serial_map(vm.space_mut(), &mut memory.frames) {
        panic!("early serial: {error:?}");
    }
    vm.activate();
    platform::early_serial_switch(&mut early_scratch);
    #[cfg(feature = "panic-tests")]
    early::test_point(params, rustos_hal::EarlyPanic::Activated);
    let stack = match memory.map_stack_painted(vm.space_mut(), KERNEL_STACK_PAGES) {
        Ok(stack) => stack,
        Err(error) => panic!("kernel stack: {error:?}"),
    };
    task::adopt_boot_stack(stack);
    let handoff = Box::into_raw(Box::new(Handoff {
        boot,
        memory,
        vm,
        entry_cycles,
        selftest: params.selftest,
    }));
    // SAFETY: `top` é o fim de uma pilha recém-mapeada e só nossa; a continuação não
    // volta, e `handoff` vem de `Box::into_raw`.
    unsafe { platform::call_on_stack(stack.top(), run_on_kernel_stack, handoff.cast()) }
}

/// # Safety
/// `arg` é o ponteiro de `Box::into_raw` de um `Handoff`, entregue uma vez só.
unsafe extern "C" fn run_on_kernel_stack(arg: *mut ()) -> ! {
    // SAFETY: contrato da função.
    let handoff = unsafe { Box::from_raw(arg.cast::<Handoff>()) };
    boot_kernel(*handoff)
}

fn boot_kernel(handoff: Handoff) -> ! {
    let Handoff {
        mut boot,
        mut memory,
        vm,
        entry_cycles,
        selftest,
    } = handoff;
    boot_time::enter(boot_time::Phase::Console);
    match platform::console(&boot, &mut memory.frames) {
        Ok(console) => console::install(console),
        Err(error) => panic!("console: {error:?}"),
    }

    let display = boot.framebuffer().and_then(|info| {
        // SAFETY: chamada única; o framebuffer veio do bootloader e só a tela o toca.
        match unsafe { screen::Screen::new(&info, &mut memory) } {
            Ok(screen) => {
                let display = display_info(&screen);
                console::install_screen(screen);
                Some(display)
            }
            Err(error) => {
                kprintln!("[rustos] screen: {error:?}");
                None
            }
        }
    });

    boot_time::enter(boot_time::Phase::Interrupts);
    let interrupts_on = interrupts::init(&boot, &mut memory);
    if platform::has_keyboard() {
        console::enable_history(&mut memory);
    }
    boot_time::enter(boot_time::Phase::Smp);
    let cpus = interrupts_on.then(|| smp::start(&boot, &mut memory));
    boot_time::enter(boot_time::Phase::Reclaim);
    // Um núcleo que não respondeu pode estar rodando na pilha ou nas tabelas do bootloader: a
    // memória dele fica onde está.
    let reclaimed = if cpus.is_some_and(|started| started.silent != 0) {
        kprintln!("[rustos] reclaim: skipped, a core never answered");
        0
    } else {
        memory.reclaim_bootloader(&mut boot)
    };
    boot_time::enter(boot_time::Phase::Setup);
    let source = random::init(&boot_marks(&boot, &memory, entry_cycles));
    report(&boot, reclaimed);
    if vm.initramfs_failed() {
        kprintln!("[rustos] initramfs: not mapped, the boot goes on without it");
    }
    let selftest = selftest::enabled(selftest);
    if selftest {
        selftest::details(&boot, &memory, display, &vm, source);
    }
    let cpus_ok = cpus.is_none_or(report_cpus);
    report_random(source);
    boot_time::enter(boot_time::Phase::SelfTests);
    // SAFETY: o mapa direto cobre todo frame utilizável, de onde vem o frame do teste.
    let frames_ok = unsafe { memory.self_test(boot.direct_map_offset()) };
    kprintln!(
        "[rustos] frame self-test: {}",
        if frames_ok { "ok" } else { "FAILED" }
    );
    let wx_ok = report_wx(&vm);
    let initramfs_ok = !selftest || selftest::initramfs(&vm);
    let exception_ok = interrupts_on && system::exception_self_test();
    let full_ok = !selftest || selftest::before_tasks();
    let tasks_ok = exception_ok && full_ok && report_tasks(&boot, &memory, selftest);
    if !frames_ok || !wx_ok || !initramfs_ok || !tasks_ok || !cpus_ok {
        rustos_hal::Cpu::halt_forever(&platform::cpu());
    }
    report_boot_time(selftest);
    kprintln!("{BOOT_OK_MARKER}");
    let mut system = system::System::new(&boot, &memory, display);
    system::run_shell(&mut system)
}

/// As linhas de tempo do boot, as últimas antes do prompt.
fn report_boot_time(selftest: bool) {
    let time = boot_time::finish();
    let ms = |us: u64| (us + 500) / 1000;
    if selftest {
        selftest::boot_phases(&time);
    }
    kprintln!(
        "[rustos] boot time: kernel {} ms to prompt (memory {}, smp {}, self-tests {}, benches {})",
        ms(time.total),
        ms(time.memory + time.reclaim),
        ms(time.smp),
        ms(time.self_tests),
        ms(time.benches)
    );
}

/// O relatório dos núcleos: a linha que o `make smoke` confere.
fn report_cpus(started: smp::Started) -> bool {
    kprintln!(
        "[rustos] cpus: {} online (found {}, limit {})",
        started.online,
        started.found,
        rustos_hal::MAX_CPUS
    );
    if started.skipped > 0 {
        kprintln!(
            "[rustos] cpus: {} left parked over the limit",
            started.skipped
        );
    }
    if started.failed != 0 {
        kprintln!("[rustos] cpus: FAILED mask {:#x}", started.failed);
    }
    started.failed == 0
}

/// Liga as tarefas e, no modo de teste, confere a preempção e o estado de SIMD de cada uma, e
/// mede a troca de contexto e o jitter de acordar (as medidas curtas do boot).
fn report_tasks(boot: &impl BootInfo, memory: &memory::Memory, selftest: bool) -> bool {
    boot_time::enter(boot_time::Phase::Setup);
    // SAFETY: o mapa direto cobre todo frame do alocador; só `task` mapeia pilhas daqui em diante.
    let summary = match unsafe { task::init(memory.frames, boot.direct_map_offset()) } {
        Ok(summary) => summary,
        Err(error) => {
            kprintln!("[rustos] tasks: {error:?}");
            return false;
        }
    };
    smp::release();
    !selftest || selftest::after_tasks(summary)
}

/// O que o boot tem de impredizível: contadores de ciclo e os tempos do bootloader.
fn boot_marks(boot: &impl BootInfo, memory: &memory::Memory, entry_cycles: u64) -> [u64; 7] {
    let timeline = boot.timeline();
    let field = |read: fn(&rustos_hal::BootTimeline) -> u64| timeline.as_ref().map_or(0, read);
    [
        entry_cycles,
        platform::cycles(),
        field(|t| t.reset_us),
        field(|t| t.loader_start_us),
        field(|t| t.kernel_handoff_us),
        platform::now_ns().unwrap_or(0),
        memory.frames.free_frames() as u64,
    ]
}

/// Diz se a imagem foi sorteada e o teto de entropia, sem dizer onde ela está (SDD 09).
fn report_kaslr(virtual_base: u64) {
    let (start, end) = vm::image_range();
    if virtual_base == redact::LINK_BASE {
        kprintln!("[rustos] kaslr: off, image at the link address");
    } else {
        kprintln!(
            "[rustos] kaslr: on, image slid by up to {} bits (4 KiB steps in 2 GiB)",
            redact::slide_bits(end - start)
        );
    }
}

/// Percorre as tabelas novas: nenhuma página gravável e executável (guidelines 09).
fn report_wx(vm: &vm::Vm) -> bool {
    let verdict = vm.check_wx();
    match verdict {
        Ok(pages) => kprintln!("[rustos] w^x: ok ({pages} pages)"),
        Err(violation) => {
            kprintln!("[rustos] w^x: violation {violation:?}");
            kprintln!("[rustos] w^x: FAILED");
        }
    }
    verdict.is_ok()
}

/// Só o aviso de fonte fraca (DEBT-0019); a fonte de hardware é do modo de teste.
fn report_random(source: random::Source) {
    if source == random::Source::Jitter {
        kprintln!("[rustos] random: jitter (weak)");
    }
}

fn display_info(screen: &screen::Screen) -> DisplayInfo {
    let (width, height) = screen.size();
    let (cols, rows) = screen.grid();
    let narrow = |value: usize| u16::try_from(value).unwrap_or(u16::MAX);
    DisplayInfo {
        width: u32::try_from(width).unwrap_or(u32::MAX),
        height: u32::try_from(height).unwrap_or(u32::MAX),
        cols: narrow(cols),
        rows: narrow(rows),
    }
}

/// O que o boot conta. O modo normal só diz o que um gate confere ou o que avisa; o resto é do
/// modo de teste (`selftest::details`) e do shell: cada linha custa tempo na serial (SDD 04).
fn report(boot: &impl BootInfo, reclaimed: u64) {
    kprintln!(
        "[rustos] RustOS {} ({})",
        env!("CARGO_PKG_VERSION"),
        platform::ARCH
    );
    if let Some(timeline) = boot.timeline() {
        kprintln!(
            "[rustos] boot timeline: firmware {} us, bootloader {} us",
            timeline.firmware_us(),
            timeline.loader_us()
        );
    }
    if let Some(image) = boot.kernel_image() {
        report_kaslr(image.virtual_base.as_u64());
        #[cfg(feature = "debug-addrs")]
        kprintln!(
            "[rustos] kernel base: {:#x} (physical {:#x})",
            image.virtual_base.as_u64(),
            image.physical_base.as_u64()
        );
    }
    kprintln!(
        "[rustos] reclaimed: {} KiB from the bootloader",
        reclaimed >> 10
    );
    let mut line = fmt::Buffer::<128>::new();
    for protection in platform::protections() {
        let mark = if protection.enabled { '+' } else { '-' };
        core::fmt::Write::write_fmt(&mut line, format_args!(" {mark}{}", protection.name)).ok();
    }
    if !line.as_str().is_empty() {
        kprintln!("[rustos] protections:{}", line.as_str());
    }
}
