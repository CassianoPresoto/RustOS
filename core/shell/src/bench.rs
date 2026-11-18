//! O comando `bench`: mostra o que o `Host` mediu. Só entra com a feature `bench`, que o kernel
//! liga junto com a `selftest` (SDD 05); sem ela o comando responde que não está no build.

use core::fmt;

use crate::ansi::{DIM, RESET};
use crate::commands::{Ctx, cpu_table};
use crate::fmtutil::{Micros, Millis};
use crate::host::BenchReport;

pub(crate) fn run(ctx: &mut Ctx<'_>) -> fmt::Result {
    let Some(report) = ctx.host.bench() else {
        return ctx.out.write_str("bench: no tasks yet\r\n");
    };
    let out = &mut ctx.out;
    write!(
        out,
        "{DIM}Context switch (ping-pong, per switch){RESET}\r\n"
    )?;
    write!(out, "  {:<14}{:>7} ns\r\n", "plain", report.switch_ns)?;
    write!(
        out,
        "  {:<14}{:>7} ns\r\n",
        "with SIMD", report.switch_simd_ns
    )?;
    if report.switch_cross_ns > 0 {
        write!(
            out,
            "  {:<14}{:>7} ns\r\n",
            "cross-core", report.switch_cross_ns
        )?;
        write!(
            out,
            "{DIM}IPI round trip between two cores{RESET}\r\n  {:<14}{:>7} ns\r\n",
            "ping-pong", report.ipi_rtt_ns
        )?;
    }
    scale_rows(out, &report)?;
    storm_rows(out, &report.storm)?;
    micro_rows(out, ctx.host.micro_bench())?;
    write!(
        out,
        "{DIM}Wake-up jitter (sleep_until, {} samples){RESET}\r\n",
        report.wake_samples
    )?;
    let row = |out: &mut dyn fmt::Write, label: &str, mean, max| {
        write!(
            out,
            "  {label:<14}mean {:>9}  max {:>9}\r\n",
            Micros(mean),
            Micros(max)
        )
    };
    row(out, "idle", report.wake_mean_ns, report.wake_max_ns)?;
    row(
        out,
        "one busy task",
        report.wake_loaded_mean_ns,
        report.wake_loaded_max_ns,
    )?;
    if report.wake_task_max_ns > 0 {
        write!(
            out,
            "{DIM}Wake-up latency (unpark of a short-slice task, same CPU){RESET}\r\n  {:<14}median {:>5} ns  mean {:>5} ns  max {:>6} ns\r\n",
            "task to task",
            report.wake_task_median_ns,
            report.wake_task_mean_ns,
            report.wake_task_max_ns
        )?;
    }
    write!(
        out,
        "{DIM}Scheduler policy (8 ready tasks, per call){RESET}\r\n"
    )?;
    write!(
        out,
        "  {:<14}{:>7} ns\r\n",
        "pick_next", report.pick_next_ns
    )?;
    write!(out, "  {:<14}{:>7} ns\r\n", "tick", report.tick_ns)?;
    cpu_table(ctx)?;
    lock_table(ctx)
}

/// Quantas vezes cada lock global foi achado preso por outra CPU, desde o boot.
fn lock_table(ctx: &mut Ctx<'_>) -> fmt::Result {
    let mut header = false;
    let mut result = Ok(());
    ctx.host.lock_contention(&mut |name, count| {
        if result.is_err() {
            return;
        }
        if !header {
            header = true;
            result = write!(
                ctx.out,
                "{DIM}Lock contention (attempts that found it held){RESET}\r\n"
            );
        }
        result = result.and_then(|()| write!(ctx.out, "  {name:<14}{count:>7}\r\n"));
    });
    result
}

/// A alocação com todas as CPUs ao mesmo tempo: frames sem e com o cache por CPU, e o heap.
fn storm_rows(out: &mut dyn fmt::Write, storm: &crate::StormSample) -> fmt::Result {
    if storm.locked_ns == 0 {
        return Ok(());
    }
    write!(
        out,
        "{DIM}Allocation storm ({} CPUs x {} rounds, total time / lock contention){RESET}\r\n",
        storm.cpus, storm.iterations
    )?;
    let row = |out: &mut dyn fmt::Write, label: &str, ns: u64, contention: u32| {
        write!(
            out,
            "  {label:<14}{:>9} ms {contention:>8}\r\n",
            Millis(ns / 1000)
        )
    };
    row(
        out,
        "frames locked",
        storm.locked_ns,
        storm.locked_contention,
    )?;
    row(
        out,
        "frames cached",
        storm.cached_ns,
        storm.cached_contention,
    )?;
    row(out, "heap", storm.heap_ns, storm.heap_contention)
}

/// Zerar memória e o gerador aleatório; sem medida, nenhuma linha.
fn micro_rows(out: &mut dyn fmt::Write, micro: Option<crate::MicroBench>) -> fmt::Result {
    let Some(micro) = micro else {
        return Ok(());
    };
    let tenths = |value: u64| (value / 10, value % 10);
    write!(out, "{DIM}Zeroing memory (per call){RESET}\r\n")?;
    for (label, value) in [
        ("1 KiB", micro.zero_kib_tenths_ns),
        ("4 KiB", micro.zero_page_tenths_ns),
        ("4 KiB in 1 MiB", micro.zero_cold_tenths_ns),
        ("4 KiB heap pair", micro.zero_pair_tenths_ns),
    ] {
        let (whole, tenth) = tenths(value);
        write!(out, "  {label:<16}{whole:>6}.{tenth} ns\r\n")?;
    }
    let (whole, tenth) = tenths(micro.random_u64_tenths_ns);
    write!(
        out,
        "{DIM}Random numbers{RESET}\r\n  {:<16}{:>6} MB/s\r\n  {:<16}{:>6} us\r\n  {:<16}{whole:>6}.{tenth} ns\r\n",
        "chacha20",
        micro.random_mb_per_s,
        "reseed",
        micro.random_reseed_ns / 1000,
        "next_u64"
    )
}

/// O tempo total de N tarefas de CPU em 1, 2 e 4 núcleos, e quantas vezes mais rápido que 1.
fn scale_rows(out: &mut dyn fmt::Write, report: &BenchReport) -> fmt::Result {
    let first = report.scale[0];
    if first.total_ns == 0 {
        return Ok(());
    }
    write!(
        out,
        "{DIM}Scale ({} CPU-bound tasks, total time){RESET}\r\n",
        first.tasks
    )?;
    for sample in report.scale.iter().filter(|sample| sample.total_ns > 0) {
        let tenths = first.total_ns * 10 / sample.total_ns;
        let plural = if sample.cpus == 1 { "cpu" } else { "cpus" };
        write!(
            out,
            "  {} {plural:<12}{:>9} ms  {}.{}x\r\n",
            sample.cpus,
            Millis(sample.total_ns / 1000),
            tenths / 10,
            tenths % 10
        )?;
    }
    Ok(())
}
