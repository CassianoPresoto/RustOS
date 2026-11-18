//! Raiz de composição do RustOS: o único crate que conhece todos os adaptadores.
//! Aqui se decide qual implementação atende cada porta; política mora em `core/`.

#![no_std]
#![no_main]

extern crate alloc;

#[cfg(not(target_os = "none"))]
compile_error!("o kernel só compila para alvos bare-metal: use `cargo xtask build`");

mod addr;
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
mod smp;
mod stack;
mod sync;
mod system;
mod task;
mod vm;

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;

use rustos_boot_limine::LimineBoot;
use rustos_hal::redact;
use rustos_hal::{BootInfo, PAGE_SIZE};
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
    /// `rustos.selftest=1`: autotestes e benchmarks completos no boot (SDD 04).
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
    report(&boot, &memory, display, reclaimed, selftest);
    if selftest {
        report_vm(&vm);
    }
    let cpus_ok = cpus.is_none_or(report_cpus);
    report_random(source, selftest);
    boot_time::enter(boot_time::Phase::SelfTests);
    // SAFETY: o mapa direto cobre todo frame utilizável, de onde vem o frame do teste.
    let frames_ok = unsafe { memory.self_test(boot.direct_map_offset()) };
    kprintln!(
        "[rustos] frame self-test: {}",
        if frames_ok { "ok" } else { "FAILED" }
    );
    let wx_ok = report_wx(&vm);
    let exception_ok = interrupts_on && system::exception_self_test();
    let full_ok = !selftest || report_full_self_tests();
    let tasks_ok = exception_ok && full_ok && report_tasks(&boot, &memory, selftest);
    if !frames_ok || !wx_ok || !tasks_ok || !cpus_ok {
        rustos_hal::Cpu::halt_forever(&platform::cpu());
    }
    report_boot_time(selftest);
    kprintln!("{BOOT_OK_MARKER}");
    let mut system = system::System::new(&boot, &memory, display);
    system::run_shell(&mut system)
}

/// O que só o modo de teste roda antes das tarefas: heap, resíduo de chave, o timer contra o
/// relógio e as medidas de zerar e do gerador. Devolve `false` se algum autoteste falhou.
fn report_full_self_tests() -> bool {
    report_key_residue();
    let heap_ok = heap::self_test();
    kprintln!(
        "[rustos] heap self-test: {}",
        if heap_ok { "ok" } else { "FAILED" }
    );
    let timer_ok = system::timer_self_test();
    boot_time::enter(boot_time::Phase::Benches);
    report_random_bench();
    report_zero_bench();
    boot_time::enter(boot_time::Phase::SelfTests);
    heap_ok && timer_ok
}

/// As linhas de tempo do boot, as últimas antes do prompt.
fn report_boot_time(selftest: bool) {
    let time = boot_time::finish();
    let ms = |us: u64| (us + 500) / 1000;
    if selftest {
        let tsc = platform::clock_error_ppm()
            .map_or_else(String::new, |ppm| format!(", tsc error at most {ppm} ppm"));
        kprintln!(
            "[rustos] boot phases: console {} us, interrupts {} us, reclaim {} us, setup {} us{tsc}",
            time.console,
            time.interrupts,
            time.reclaim,
            time.setup
        );
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
    if selftest {
        kprintln!(
            "[rustos] fpu: {}, {} bytes per task, {}",
            summary.mode,
            summary.bytes,
            summary.components
        );
    }
    smp::release();
    if !selftest {
        return true;
    }
    boot_time::enter(boot_time::Phase::SelfTests);
    kprintln!(
        "[rustos] sched: fifo+eevdf per cpu, {} us base slice, {} tasks max, {} cpus, timer {}",
        rustos_sched::eevdf::BASE_SLICE_NS / 1000,
        task::MAX_TASKS - rustos_hal::MAX_CPUS,
        task::active_cpus().count_ones(),
        platform::timer_mode()
    );
    let (irqs, window_ms) = bench::idle_timer_irqs();
    kprintln!("[rustos] idle: {irqs} timer irqs in {window_ms} ms (tickless)");
    let preempted = bench::simd_self_test();
    match preempted {
        Some(count) => {
            kprintln!("[rustos] task self-test: ok (simd state intact, {count} switches)");
        }
        None => kprintln!("[rustos] task self-test: FAILED"),
    }
    let smp_ok = if let Some((migrations, turns)) = bench::smp_self_test() {
        kprintln!("[rustos] smp self-test: ok ({turns} turns, {migrations} migrations)");
        true
    } else {
        kprintln!("[rustos] smp self-test: FAILED");
        false
    };
    boot_time::enter(boot_time::Phase::Benches);
    let measured = bench::run(bench::BOOT);
    boot_time::enter(boot_time::Phase::SelfTests);
    match measured {
        Some(report) => {
            kprintln!(
                "[rustos] switch: {} ns ({} with simd, {} cross-core), ipi round trip {} ns, pick_next {} ns, tick {} ns, wake: mean {} ns, max {} ns, {} samples",
                report.switch_ns,
                report.switch_simd_ns,
                report.switch_cross_ns,
                report.ipi_rtt_ns,
                report.pick_next_ns,
                report.tick_ns,
                report.wake_mean_ns,
                report.wake_max_ns,
                report.wake_samples
            );
            report_scale(&report);
            report_storm(&report.storm);
        }
        None => kprintln!("[rustos] switch: not measured"),
    }
    let stacks_ok = task::log_stacks();
    preempted.is_some() && smp_ok && stacks_ok
}

/// O tempo total das tarefas de CPU em 1, 2 e 4 núcleos, em microssegundos.
fn report_scale(report: &rustos_shell::BenchReport) {
    let mut line = fmt::Buffer::<128>::new();
    for sample in report.scale.iter().filter(|sample| sample.total_ns > 0) {
        core::fmt::Write::write_fmt(
            &mut line,
            format_args!(" {} cpu: {} us,", sample.cpus, sample.total_ns / 1000),
        )
        .ok();
    }
    if !line.as_str().is_empty() {
        let tasks = report.scale[0].tasks;
        kprintln!(
            "[rustos] scale: {tasks} tasks,{}",
            line.as_str().trim_end_matches(',')
        );
    }
}

/// A contenção da memória: frames pelo lock do buddy, com o cache por CPU, e o heap.
fn report_storm(storm: &rustos_shell::StormSample) {
    if storm.locked_ns > 0 {
        kprintln!(
            "[rustos] alloc storm: {} cpus x {}, frames locked {} us ({} contended), cached {} us ({}), heap {} us ({})",
            storm.cpus,
            storm.iterations,
            storm.locked_ns / 1000,
            storm.locked_contention,
            storm.cached_ns / 1000,
            storm.cached_contention,
            storm.heap_ns / 1000,
            storm.heap_contention
        );
    }
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

fn report_vm(vm: &vm::Vm) {
    let stats = vm.stats();
    kprintln!(
        "[rustos] vm: direct map in {} 2M and {} 4K pages, {} KiB of tables, stack {} KiB",
        stats.large_pages,
        stats.small_pages,
        (stats.table_frames * PAGE_SIZE) >> 10,
        (KERNEL_STACK_PAGES * PAGE_SIZE) >> 10
    );
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

/// A fonte de entropia; no modo normal só o aviso de fonte fraca (DEBT-0019).
fn report_random(source: random::Source, verbose: bool) {
    match source {
        random::Source::Hardware(name) if verbose => kprintln!("[rustos] random: {name}"),
        random::Source::Hardware(_) => {}
        random::Source::Jitter => kprintln!("[rustos] random: jitter (weak)"),
    }
}

fn report_random_bench() {
    if let Some(bench) = random::benchmark() {
        kprintln!(
            "[rustos] random bench: chacha20 {} MB/s, reseed {} us, next_u64 {}.{} ns",
            bench.mb_per_s,
            bench.reseed_ns / 1000,
            bench.u64_tenths_ns / 10,
            bench.u64_tenths_ns % 10
        );
    }
}

/// Procura cópias da chave na pilha depois de semear e de um pedido (SDD 09). Heurística
/// que depende do código gerado: só imprime, o `make smoke` reprova pelo `FAILED`.
fn report_key_residue() {
    use rustos_hal::Cpu;
    // A IRQ empilha o quadro dela abaixo do `sp`, sobre o que a varredura lê: com ela
    // ligada, a sonda falha e o resíduo some ao acaso (visto no smoke do CI, TCG lento).
    let cpu = platform::cpu();
    let were_enabled = cpu.save_and_disable_interrupts();
    let residue = random::stack_residue(false);
    let probe = random::stack_residue(true);
    cpu.restore_interrupts(were_enabled);
    let sees = probe.states == random::PLANTED;
    let clean = residue == random::Residue::default();
    kprintln!(
        "[rustos] key residue: {} chacha states, {} key copies on the stack, probe {} of {}: {}",
        residue.states,
        residue.keys,
        probe.states,
        random::PLANTED,
        if clean && sees { "ok" } else { "FAILED" }
    );
}

fn report_zero_bench() {
    if let Some(bench) = heap::zero_benchmark() {
        let tenths = |value: u64| (value / 10, value % 10);
        let (kib, page) = (tenths(bench.kib), tenths(bench.page));
        let (cold, pair) = (tenths(bench.cold_page), tenths(bench.pair));
        kprintln!(
            "[rustos] zero bench: 1 KiB {}.{} ns, 4 KiB {}.{} ns, 4 KiB over 1 MiB {}.{} ns, 4 KiB alloc+free {}.{} ns",
            kib.0,
            kib.1,
            page.0,
            page.1,
            cold.0,
            cold.1,
            pair.0,
            pair.1
        );
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

/// O que o boot conta. O modo normal só diz o que um gate confere ou o que avisa; o resto
/// (`verbose`) é do modo de teste e do shell: cada linha custa tempo na serial (SDD 04).
fn report(
    boot: &impl BootInfo,
    memory: &memory::Memory,
    display: Option<DisplayInfo>,
    reclaimed: u64,
    verbose: bool,
) {
    kprintln!(
        "[rustos] RustOS {} ({})",
        env!("CARGO_PKG_VERSION"),
        platform::ARCH
    );
    if let Some(loader) = boot.bootloader().filter(|_| verbose) {
        kprintln!("[rustos] bootloader: {} {}", loader.name, loader.version);
    }
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
    let summary = memory.summary;
    if verbose {
        kprintln!(
            "[rustos] memory: {} KiB usable, {} KiB other, {} regions",
            summary.usable_bytes >> 10,
            summary.other_bytes >> 10,
            summary.regions
        );
    }
    kprintln!(
        "[rustos] reclaimed: {} KiB from the bootloader",
        reclaimed >> 10
    );
    if verbose {
        report_frames_and_screen(memory, display);
    }
    let mut line = fmt::Buffer::<128>::new();
    for protection in platform::protections() {
        let mark = if protection.enabled { '+' } else { '-' };
        core::fmt::Write::write_fmt(&mut line, format_args!(" {mark}{}", protection.name)).ok();
    }
    if !line.as_str().is_empty() {
        kprintln!("[rustos] protections:{}", line.as_str());
    }
}

fn report_frames_and_screen(memory: &memory::Memory, display: Option<DisplayInfo>) {
    kprintln!(
        "[rustos] frames: {} free of {} tracked, metadata {} bytes",
        memory.frames.free_frames(),
        memory.frames.total_frames(),
        memory.metadata_bytes
    );
    match display {
        Some(display) => {
            let (alt, history) = console::screen_modes().unwrap_or_default();
            kprintln!(
                "[rustos] screen: {}x{}, grid {}x{}, alt {}, history {} lines",
                display.width,
                display.height,
                display.cols,
                display.rows,
                if alt { "on" } else { "off" },
                history
            );
        }
        None => kprintln!("[rustos] screen: none"),
    }
}
