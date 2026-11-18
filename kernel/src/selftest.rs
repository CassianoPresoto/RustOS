//! O modo de teste (`rustos.selftest=1`): autotestes e medidas do boot. Só existe na imagem com
//! a feature `selftest`; a enxuta usa `selftest_off.rs` no lugar (SDD 04, SDD 05).

use alloc::format;
use alloc::string::String;

use rustos_hal::{BootInfo, Cpu, PAGE_SIZE};
use rustos_shell::DisplayInfo;

use crate::frames::Frames;
use crate::memory::Memory;
use crate::platform::FpuSummary;
use crate::{
    KERNEL_STACK_PAGES, bench, boot_time, console, fmt, heap, init, interrupts, platform, process,
    random, task, vm,
};

/// O boot de teste foi pedido e esta imagem o tem.
pub fn enabled(requested: bool) -> bool {
    requested
}

/// As linhas de estado que só o modo de teste imprime, depois das comuns.
pub fn details(
    boot: &impl BootInfo,
    memory: &Memory,
    display: Option<DisplayInfo>,
    vm: &vm::Vm,
    source: random::Source,
) {
    if let Some(loader) = boot.bootloader() {
        kprintln!("[rustos] bootloader: {} {}", loader.name, loader.version);
    }
    let summary = memory.summary;
    kprintln!(
        "[rustos] memory: {} KiB usable, {} KiB other, {} regions",
        summary.usable_bytes >> 10,
        summary.other_bytes >> 10,
        summary.regions
    );
    report_frames_and_screen(memory, display);
    let stats = vm.stats();
    kprintln!(
        "[rustos] vm: direct map in {} 2M and {} 4K pages, {} KiB of tables, stack {} KiB",
        stats.large_pages,
        stats.small_pages,
        (stats.table_frames * PAGE_SIZE) >> 10,
        (KERNEL_STACK_PAGES * PAGE_SIZE) >> 10
    );
    if let random::Source::Hardware(name) = source {
        kprintln!("[rustos] random: {name}");
    }
    crate::mitigation::details();
}

/// O `init` e o programa de teste que o `xtask image` põe no initramfs da imagem com `selftest`.
const PROBE_NAMES: [&[u8]; 2] = [b"init", b"big"];
const ELF_MAGIC: &[u8] = b"\x7fELF";

/// Lê o initramfs pelo `core/cpio`, confere que cada programa esperado está nele e é um ELF, e que
/// o mapeamento é só de leitura. Sem módulo, diz `none` e segue (o `smoke` é quem exige a linha).
pub fn initramfs(vm: &vm::Vm) -> bool {
    let Some(archive) = vm.initramfs() else {
        let failed = vm.initramfs_failed();
        kprintln!(
            "[rustos] initramfs: {}",
            if failed { "FAILED" } else { "none" }
        );
        return !failed;
    };
    let (mut files, mut found) = (0_usize, [false; PROBE_NAMES.len()]);
    for entry in rustos_cpio::Reader::new(archive) {
        let Ok(entry) = entry else {
            kprintln!("[rustos] initramfs: FAILED (unreadable archive)");
            return false;
        };
        files += 1;
        for (name, seen) in PROBE_NAMES.iter().zip(&mut found) {
            *seen |= entry.name == *name && entry.data.starts_with(ELF_MAGIC);
        }
    }
    let read_only = vm.initramfs_is_read_only();
    let programs = found.iter().all(|&seen| seen);
    if !programs || !read_only {
        kprintln!("[rustos] initramfs: FAILED (programs {programs}, read-only {read_only})");
        return false;
    }
    kprintln!("[rustos] initramfs: {files} files, {} bytes", archive.len());
    true
}

fn report_frames_and_screen(memory: &Memory, display: Option<DisplayInfo>) {
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

/// O que o modo de teste roda antes das tarefas: heap, resíduo de chave, o timer contra o
/// relógio e as medidas de zerar e do gerador. Devolve `false` se algum autoteste falhou.
pub fn before_tasks() -> bool {
    report_key_residue();
    let heap_ok = heap::self_test();
    kprintln!(
        "[rustos] heap self-test: {}",
        if heap_ok { "ok" } else { "FAILED" }
    );
    let timer_ok = timer_self_test();
    boot_time::enter(boot_time::Phase::Benches);
    report_random_bench();
    report_zero_bench();
    boot_time::enter(boot_time::Phase::SelfTests);
    heap_ok && timer_ok
}

/// Com as tarefas ligadas: a preempção e o estado de SIMD de cada uma, a migração entre núcleos,
/// a troca de contexto, o jitter de acordar e as pilhas. Devolve `false` se algum falhou.
pub fn after_tasks(summary: FpuSummary, frames: Frames, map_offset: u64, vm: &vm::Vm) -> bool {
    boot_time::enter(boot_time::Phase::Init);
    let init_ok = init::run(vm, frames, map_offset);
    boot_time::enter(boot_time::Phase::SelfTests);
    kprintln!(
        "[rustos] fpu: {}, {} bytes per task, {}",
        summary.mode,
        summary.bytes,
        summary.components
    );
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
    let user_cases = process::self_test(frames, map_offset, vm.initramfs());
    let user_ok = init_ok && user_cases;
    let stacks_ok = task::log_stacks();
    preempted.is_some() && smp_ok && user_ok && stacks_ok
}

/// O que a linha `boot time:` não mostra: o tempo das fases e o erro da calibração do TSC.
pub fn boot_phases(time: &boot_time::Summary) {
    let tsc = platform::clock_error_ppm()
        .map_or_else(String::new, |ppm| format!(", tsc error at most {ppm} ppm"));
    kprintln!(
        "[rustos] boot phases: console {} us, interrupts {} us, reclaim {} us, setup {} us, init {} us{tsc}",
        time.console,
        time.interrupts,
        time.reclaim,
        time.setup,
        time.init
    );
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

/// Tiques que o autoteste espera: 50 ms a 100 Hz. Fora de 25–200 ms, a calibração
/// errou; sem tique em 1 s, o timer não chega.
const SELF_TEST_MS: core::ops::RangeInclusive<u64> = 25..=200;
const SELF_TEST_GIVE_UP_NS: u64 = 1_000_000_000;
const SELF_TEST_TICKS: u64 = 5;

/// Tiques do timer contados contra o relógio: 50 ms de espera, antes de o escalonador ligar.
pub fn timer_self_test() -> bool {
    let cpu = platform::cpu();
    let elapsed = || platform::now_ns().unwrap_or(0);
    let (start_ticks, start_ns) = (interrupts::ticks(), elapsed());
    while interrupts::ticks() < start_ticks + SELF_TEST_TICKS
        && elapsed().saturating_sub(start_ns) < SELF_TEST_GIVE_UP_NS
    {
        cpu.save_and_disable_interrupts();
        cpu.enable_interrupts_and_wait();
    }
    let elapsed_ms = elapsed().saturating_sub(start_ns) / 1_000_000;
    let ticked = interrupts::ticks() >= start_ticks + SELF_TEST_TICKS;
    let timer_ok = ticked && SELF_TEST_MS.contains(&elapsed_ms);
    kprintln!(
        "[rustos] timer: {} Hz, {SELF_TEST_TICKS} ticks in {elapsed_ms} ms {}",
        platform::TIMER_HZ,
        if timer_ok { "ok" } else { "FAILED" }
    );
    timer_ok
}
