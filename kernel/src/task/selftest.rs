//! A prova e a linha `stacks:` do modo de teste, só com a feature `selftest`.

use core::sync::atomic::Ordering;

use rustos_sched::SchedParams;

use super::{FIRST_TASK, Spawn, Stack, TASK_PEAK, TASKS, has_exited, slot, spawn, yield_now};

/// Bytes que a tarefa de prova escreve na pilha; o pico medido tem de cobri-los.
const PROBE_BYTES: usize = 3000;

fn probe_main(_: usize) {
    let mut buffer = [1u8; PROBE_BYTES];
    core::hint::black_box(&mut buffer);
}

/// Autoteste da marca d'água: a tarefa escreve `PROBE_BYTES` e o pico lido depois do `exit`
/// tem de ser pelo menos isso. Devolve o pico, se passou.
fn stack_probe() -> Option<usize> {
    let id = spawn(Spawn {
        name: "probe",
        entry: probe_main,
        arg: 0,
        simd: false,
        params: SchedParams::fair(0),
        affinity: 0,
    })
    .ok()?;
    while !has_exited(id) {
        yield_now();
    }
    let peak = slot(id.as_u32() as usize)
        .meta
        .lock()
        .stack
        .map_or(0, Stack::peak);
    (peak >= PROBE_BYTES).then_some(peak)
}

/// A linha `stacks:` do boot, que o SDD 15 pede para remedir quando o uso de pilha mudar. Reprova
/// (`FAILED`, que o smoke recusa) se a prova falha ou se algum pico passa de 75% da pilha.
pub fn log_stacks() -> bool {
    let report = stack_report();
    let probe = stack_probe();
    let over = |peak: usize, bytes: usize| peak * 4 > bytes * 3;
    let ok = probe.is_some()
        && !over(report.idle_peak, report.idle_bytes)
        && !over(report.task_peak, report.task_bytes)
        && !over(report.shell_peak, report.shell_bytes);
    let kib = |bytes: usize| (bytes / 1024, bytes % 1024 * 10 / 1024);
    let (idle, task, shell) = (
        kib(report.idle_peak),
        kib(report.task_peak),
        kib(report.shell_peak),
    );
    kprintln!(
        "[rustos] stacks: idle peak {}.{} KiB of {}, task peak {}.{} KiB of {}, shell peak {}.{} KiB of {}, probe {} of {PROBE_BYTES} B {}",
        idle.0,
        idle.1,
        report.idle_bytes / 1024,
        task.0,
        task.1,
        report.task_bytes / 1024,
        shell.0,
        shell.1,
        report.shell_bytes / 1024,
        probe.unwrap_or(0),
        if ok { "ok" } else { "FAILED" }
    );
    ok
}

/// Pico e tamanho das pilhas por tipo de tarefa, em bytes.
#[derive(Clone, Copy, Debug, Default)]
pub struct StackReport {
    pub idle_peak: usize,
    pub idle_bytes: usize,
    pub task_peak: usize,
    pub task_bytes: usize,
    pub shell_peak: usize,
    pub shell_bytes: usize,
}

/// O maior pico de cada tipo desde o boot: as ociosas, o shell (a pilha do boot) e as tarefas,
/// as já encerradas incluídas (o slot guarda a marca até ser reusado).
pub fn stack_report() -> StackReport {
    let mut report = StackReport {
        task_peak: TASK_PEAK.load(Ordering::Relaxed),
        ..StackReport::default()
    };
    for (id, task) in TASKS.iter().enumerate() {
        let Some(stack) = task.meta.lock().stack else {
            continue;
        };
        let peak = stack.peak();
        let (max, bytes) = match id {
            _ if id < FIRST_TASK => (&mut report.idle_peak, &mut report.idle_bytes),
            FIRST_TASK => (&mut report.shell_peak, &mut report.shell_bytes),
            _ => (&mut report.task_peak, &mut report.task_bytes),
        };
        *max = (*max).max(peak);
        *bytes = (*bytes).max(stack.bytes());
    }
    report
}
