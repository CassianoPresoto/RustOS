//! O autoteste de usuário (só na imagem com a feature `selftest`): o programa embutido no kernel
//! roda em anel 3 ou EL0 nos casos `hello`, falha contida e laço de syscall nula, e a limpeza da
//! pilha do kernel na volta ao usuário se confere por um resíduo plantado (4a); o programa `big` do
//! initramfs passa o carregador por segmentos grandes e o carregador recusa o que é inválido (4b).

use core::hint::black_box;
use core::ptr;
use core::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

use rustos_hal::abi::{ProcessEvent, event_kind, fault_kind};

use super::{Program, Source, fault_peak, last_event, spawn};
use crate::frames::Frames;
use crate::platform::user_blob::{Case, image, offset};
use crate::{loader, platform, scrub, task};

/// Quanto o autoteste espera um processo acabar, e o passo da espera.
const WAIT_NS: u64 = 15_000_000_000;
const POLL_NS: u64 = 1_000_000;
/// Chamadas `nop` do laço de medida. O TCG do x86_64 é lento: só compara o RustOS consigo mesmo.
const LOOP_CALLS: u64 = if cfg!(target_arch = "x86_64") {
    20_000
} else {
    200_000
};
const RUNS: usize = 5;
/// Frames que ficam fora da conta de "voltou tudo": a pilha de kernel que o slot da tarefa guarda e
/// as páginas que o heap ganhou.
const SLACK_FRAMES: usize = 64;

/// Um endereço do kernel que o processo tenta ler: a página existe e é do kernel.
static KERNEL_PROBE: u64 = 0x5253_5441_434b_2121;

/// Palavras que `plant` deixa na pilha e `scan` procura: 2 KiB.
const PLANTED_WORDS: usize = 256;
const MAGIC: u64 = 0x4b45_5953_5441_5445;
const OFF: u8 = 0;
const PLANT_NEXT: u8 = 1;
const SCAN_NEXT: u8 = 2;
static PROBE: AtomicU8 = AtomicU8::new(OFF);
static FOUND: AtomicUsize = AtomicUsize::new(0);

/// Vezes que o kernel rodou com `EFLAGS.AC` ligado, vistas na volta ao usuário e na troca.
static AC_LEAKS: AtomicUsize = AtomicUsize::new(0);

/// Chamado na volta ao usuário e depois de cada troca de tarefa: o kernel nunca roda com `AC`.
pub fn check_ac() {
    if platform::ac_set() {
        AC_LEAKS.fetch_add(1, Ordering::Relaxed);
    }
}

/// O processo liga `NT` e `AC` por `popf` e faz syscalls durante 8 ms, na CPU 0, onde o timer
/// do próprio autoteste o interrompe: sobrevive, e o kernel nunca roda com `AC`. O caso
/// `privileged` liga `AC` e leva um `#GP`, o caminho que a QEMU não limpa sozinho.
fn flags_survive(frames: Frames, map_offset: u64) -> bool {
    let start = (platform::now_ns(), platform::cycles());
    while platform::now_ns()
        .zip(start.0)
        .is_some_and(|(now, from)| now < from + 1_000_000)
    {}
    let per_ms = platform::cycles().saturating_sub(start.1);
    AC_LEAKS.store(0, Ordering::Relaxed);
    let result = run_case(frames, map_offset, Case::FlagsAc, per_ms * 8, 1);
    exited_with(result, 0) && AC_LEAKS.load(Ordering::Relaxed) == 0
}

/// O gancho do `nop`: na primeira chamada planta um padrão fundo na pilha do kernel, na segunda
/// varre a mesma profundidade. Sem a limpeza na volta ao usuário o padrão ainda está lá.
pub fn on_nop() {
    match PROBE.load(Ordering::Relaxed) {
        PLANT_NEXT => {
            plant();
            PROBE.store(SCAN_NEXT, Ordering::Relaxed);
        }
        SCAN_NEXT => {
            FOUND.store(scan(), Ordering::Relaxed);
            PROBE.store(OFF, Ordering::Relaxed);
        }
        _ => {}
    }
}

#[inline(never)]
fn plant() {
    let mut marks = [MAGIC; PLANTED_WORDS];
    black_box(&mut marks);
}

/// Conta as palavras do padrão na pilha abaixo da própria função.
#[inline(never)]
fn scan() -> usize {
    let probe = 0_u64;
    let here = ptr::from_ref(&probe).expose_provenance();
    let (low, high) = (here - PLANTED_WORDS * 8, here - 256);
    (low..high)
        .step_by(8)
        .filter(|&address| {
            // SAFETY: a faixa está na pilha desta tarefa, mapeada, abaixo do quadro que lê.
            unsafe { ptr::with_exposed_provenance::<u64>(address).read_volatile() == MAGIC }
        })
        .count()
}

/// Roda um caso até o processo acabar e devolve o evento dele.
fn run_case(
    frames: Frames,
    offset_of_map: u64,
    case: Case,
    arg0: u64,
    affinity: u32,
) -> Option<(u32, ProcessEvent)> {
    let program = Program {
        code: image(),
        entry_offset: offset(case),
        arg0,
    };
    run_source(frames, offset_of_map, Source::Flat(program), affinity)
}

/// Roda `source` até o processo acabar e devolve o evento dele.
fn run_source(
    frames: Frames,
    map_offset: u64,
    source: Source<'_>,
    affinity: u32,
) -> Option<(u32, ProcessEvent)> {
    let spawned = spawn(frames, map_offset, source, affinity).ok()?;
    let deadline = platform::now_ns()? + WAIT_NS;
    while !task::has_exited(spawned.task) {
        let now = platform::now_ns()?;
        if now > deadline {
            return None;
        }
        task::sleep_until(now + POLL_NS);
    }
    last_event().filter(|(last, _)| *last == spawned.pid)
}

/// O `big` do initramfs (mais de 2 MiB, com `.rodata`, `.data` e `.bss` de muitas páginas) roda e sai
/// com 0: o carregador copia, zera e mapeia segmentos grandes e o `-C code-model=small` aguenta.
fn big_program_runs(frames: Frames, map_offset: u64, archive: Option<&[u8]>) -> bool {
    let Some(file) = archive.and_then(|archive| loader::find(archive, b"big").ok()) else {
        return false;
    };
    let free_before = frames.free_frames();
    let result = run_source(frames, map_offset, Source::Elf(file), 0);
    let returned = frames.free_frames() + SLACK_FRAMES >= free_before;
    kprintln!("[rustos] user: big ran ({} KiB of file)", file.len() >> 10);
    exited_with(result, 0) && returned
}

/// O carregador recusa o que o `core/elf` recusa e não deixa frame para trás: arquivo cortado na
/// tabela de program headers,
/// máquina de outra ISA, sem magic e vazio. `frames` volta ao que era.
fn loader_refuses_bad_executables(frames: Frames, map_offset: u64, archive: Option<&[u8]>) -> bool {
    let Some(good) = archive.and_then(|archive| loader::find(archive, b"init").ok()) else {
        return false;
    };
    let mut other_machine = good.to_vec();
    other_machine[18] ^= 0xFF;
    let mut bad_magic = good.to_vec();
    bad_magic[0] = 0;
    let free_before = frames.free_frames();
    let refused = [&good[..100], &other_machine, &bad_magic, &[]]
        .into_iter()
        .all(|file| spawn(frames, map_offset, Source::Elf(file), 0).is_err());
    refused && frames.free_frames() + SLACK_FRAMES >= free_before
}

fn exited_with(result: Option<(u32, ProcessEvent)>, code: i64) -> bool {
    result.is_some_and(|(_, event)| event.kind == event_kind::EXIT && event.code == code)
}

fn faulted(result: Option<(u32, ProcessEvent)>) -> Option<ProcessEvent> {
    result
        .map(|(_, event)| event)
        .filter(|event| event.kind == event_kind::FAULT)
}

/// Décimos de ns por chamada de uma execução do laço, na CPU `cpu` (sem migração no meio).
fn sample_tenths_ns(frames: Frames, map_offset: u64, cpu: usize) -> Option<u64> {
    let (_, event) = run_case(frames, map_offset, Case::NullLoop, LOOP_CALLS, 1 << cpu)?;
    if event.kind != event_kind::EXIT {
        return None;
    }
    Some(platform::cycles_to_ns(event.code.cast_unsigned()) * 10 / LOOP_CALLS)
}

/// As medianas, em décimos de ns, do laço com a limpeza ligada e desligada e, onde a ISA tem, com
/// a saída lenta. As configurações se alternam a cada rodada, para a deriva da máquina cair
/// igual nelas.
#[derive(Clone, Copy, Debug)]
struct Medians {
    clean: u64,
    dirty: u64,
    slow_return: Option<u64>,
}

fn median(mut samples: [u64; RUNS]) -> u64 {
    samples.sort_unstable();
    samples[RUNS / 2]
}

fn null_loop_medians(frames: Frames, map_offset: u64) -> Option<Medians> {
    let cpu = usize::from(task::active_cpus() & 2 != 0);
    let (mut clean, mut dirty, mut slow) = ([0; RUNS], [0; RUNS], [0; RUNS]);
    for round in 0..RUNS {
        scrub::set_enabled(true);
        clean[round] = sample_tenths_ns(frames, map_offset, cpu)?;
        scrub::set_enabled(false);
        dirty[round] = sample_tenths_ns(frames, map_offset, cpu)?;
        scrub::set_enabled(true);
        if platform::HAS_FAST_RETURN {
            platform::set_fast_return(false);
            let sample = sample_tenths_ns(frames, map_offset, cpu);
            platform::set_fast_return(true);
            slow[round] = sample?;
        }
    }
    Some(Medians {
        clean: median(clean),
        dirty: median(dirty),
        slow_return: platform::HAS_FAST_RETURN.then(|| median(slow)),
    })
}

/// O resíduo achado depois de plantar na primeira chamada e varrer na segunda.
fn residue(frames: Frames, map_offset: u64) -> Option<usize> {
    FOUND.store(usize::MAX, Ordering::Relaxed);
    PROBE.store(PLANT_NEXT, Ordering::Relaxed);
    run_case(frames, map_offset, Case::NullLoop, 16, 0)?;
    PROBE.store(OFF, Ordering::Relaxed);
    Some(FOUND.load(Ordering::Relaxed)).filter(|&found| found != usize::MAX)
}

/// O `hello` passou um ponteiro não mapeado ao `debug_write`: a falta da cópia foi recuperada no
/// kernel (onde há `AC`) e o tratador rodou com `AC` limpo, vindo do kernel dentro da janela do `stac`.
fn copy_fixup_ran_with_ac_clear() -> bool {
    platform::copy_fixup_stats().is_none_or(|(fixups, with_ac)| fixups > 0 && with_ac == 0)
}

/// O `hello` roda e sai com 0 em cada CPU ativa: a entrada de syscall e o espaço de usuário de
/// cada núcleo (os MSRs de `SYSCALL` ou o `TTBR0_EL1`) são conferidos um a um.
fn hello_on_every_cpu(frames: Frames, map_offset: u64) -> bool {
    let active = task::active_cpus();
    (0..rustos_hal::MAX_CPUS)
        .filter(|cpu| active >> cpu & 1 != 0)
        .all(|cpu| {
            let result = run_case(frames, map_offset, Case::Hello, 0, 1 << cpu);
            exited_with(result, 0)
        })
}

/// Os casos do programa embutido. `false` se algum falhou.
pub fn self_test(frames: Frames, map_offset: u64, archive: Option<&[u8]>) -> bool {
    let hello = hello_on_every_cpu(frames, map_offset) && copy_fixup_ran_with_ac_clear();
    let probe = ptr::addr_of!(KERNEL_PROBE).expose_provenance() as u64;
    let kernel = faulted(run_case(frames, map_offset, Case::FaultMemory, probe, 0));
    let kernel_ok = kernel.is_some_and(|event| event.fault_kind == fault_kind::PAGE_FAULT);
    AC_LEAKS.store(0, Ordering::Relaxed);
    let privileged = faulted(run_case(frames, map_offset, Case::FaultPrivileged, 0, 0)).is_some()
        && AC_LEAKS.load(Ordering::Relaxed) == 0;
    let trap = faulted(run_case(frames, map_offset, Case::FaultTrap, 0, 0)).is_some();
    let stack = faulted(run_case(frames, map_offset, Case::FaultBadStack, 0, 0)).is_some();
    let flags = flags_survive(frames, map_offset);
    let big = big_program_runs(frames, map_offset, archive);
    let refused = loader_refuses_bad_executables(frames, map_offset, archive);
    let (syscall_depth, fault_depth) = (scrub::deepest(), fault_peak());
    scrub::set_measuring(false);
    let clean_found = residue(frames, map_offset);
    scrub::set_enabled(false);
    let dirty_found = residue(frames, map_offset);
    scrub::set_enabled(true);
    let medians = null_loop_medians(frames, map_offset);
    scrub::set_measuring(true);
    let scrub_ok = clean_found == Some(0) && dirty_found.is_some_and(|found| found > 0);
    if let Some(medians) = medians {
        kprintln!(
            "[rustos] user: null syscall {}.{} ns with the stack clean, {}.{} ns without ({LOOP_CALLS} calls, median of {RUNS} alternating runs)",
            medians.clean / 10,
            medians.clean % 10,
            medians.dirty / 10,
            medians.dirty % 10
        );
        if let Some(slow) = medians.slow_return {
            kprintln!(
                "[rustos] user: null syscall {}.{} ns leaving by iretq instead of sysret",
                slow / 10,
                slow % 10
            );
        }
    }
    kprintln!(
        "[rustos] user: stack clean left {} marked words, {} without it; kernel stack depth {} B on syscall, {} B on fault",
        clean_found.unwrap_or(usize::MAX),
        dirty_found.unwrap_or(usize::MAX),
        syscall_depth,
        fault_depth
    );
    let ok = hello
        && kernel_ok
        && privileged
        && trap
        && stack
        && flags
        && scrub_ok
        && big
        && refused
        && medians.is_some();
    kprintln!(
        "[rustos] user self-test: {} (hello {hello}, kernel page {kernel_ok}, privileged {privileged}, trap {trap}, bad stack {stack}, flags {flags}, stack clean {scrub_ok}, big {big}, bad executables {refused})",
        if ok { "ok" } else { "FAILED" }
    );
    ok
}
