//! O autoteste de usuário (só na imagem com a feature `selftest`): o programa `cases` do initramfs
//! roda em anel 3 ou EL0 nos casos `hello`, falha contida e laço de syscall nula, e a limpeza da
//! pilha do kernel na volta ao usuário se confere por um resíduo plantado (4a); o programa `big` do
//! initramfs passa o carregador por segmentos grandes e o carregador recusa o que é inválido (4b).

use core::hint::black_box;
use core::ptr;
use core::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

use alloc::string::ToString;

use rustos_hal::abi::{ProcessEvent, USER_TOP, event_kind, fault_access, fault_kind};

mod demand;
mod procs;
mod tags;

use super::{Options, Source, fault_peak, last_event, spawn, spawn_as, spawn_keeping, spawn_with};
use crate::frames::Frames;
use crate::uspace::{Resolved, UserSpace, set_frame_budget};
use crate::{asid, console, framebuffer, loader, platform, scrub, task};
use rustos_mm::vmar::Access;

/// Quanto o autoteste espera um processo acabar, e o passo da espera.
pub(super) const WAIT_NS: u64 = 15_000_000_000;
const POLL_NS: u64 = 1_000_000;
/// Chamadas `nop` do laço de medida. O TCG do x86_64 é lento: só compara o RustOS consigo mesmo.
const LOOP_CALLS: u64 = if cfg!(target_arch = "x86_64") {
    20_000
} else {
    200_000
};
const RUNS: usize = 5;

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
fn flags_survive(frames: Frames, map_offset: u64, archive: Option<&[u8]>) -> bool {
    let start = (platform::now_ns(), platform::cycles());
    while platform::now_ns()
        .zip(start.0)
        .is_some_and(|(now, from)| now < from + 1_000_000)
    {}
    let per_ms = platform::cycles().saturating_sub(start.1);
    AC_LEAKS.store(0, Ordering::Relaxed);
    let result = run_case(frames, map_offset, archive, "flags-ac", per_ms * 8, 1);
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

/// Roda o caso `case` do `cases` do initramfs com `arg0` até o processo acabar e devolve o evento
/// dele; `None` se o initramfs não tem `cases` ou o processo não acaba.
fn run_case(
    frames: Frames,
    map_offset: u64,
    archive: Option<&[u8]>,
    case: &str,
    arg0: u64,
    affinity: u32,
) -> Option<(u32, ProcessEvent)> {
    let file = loader::find(archive?, b"cases").ok()?;
    let arg0 = arg0.to_string();
    let arguments: [&[u8]; 3] = [b"cases", case.as_bytes(), arg0.as_bytes()];
    let spawned = spawn_as(
        frames,
        map_offset,
        Source::Elf(file),
        affinity,
        "cases",
        &arguments,
    )
    .ok()?;
    finish(&spawned)
}

/// Roda `source` até o processo acabar e devolve o evento dele.
fn run_source(
    frames: Frames,
    map_offset: u64,
    source: Source<'_>,
    affinity: u32,
) -> Option<(u32, ProcessEvent)> {
    let spawned = spawn(frames, map_offset, source, affinity).ok()?;
    finish(&spawned)
}

/// Espera o processo acabar e devolve o evento dele.
pub(super) fn finish(spawned: &super::Spawned) -> Option<(u32, ProcessEvent)> {
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

/// A afinidade que prende o processo à CPU de quem testa: o frame que ele devolve cai no cache de
/// frames desta CPU, que `free_frames` conta, e a conta fica exata.
fn here() -> u32 {
    1 << crate::percpu::pinned(crate::percpu::PerCpu::index)
}

/// Os frames que um processo bom que acabou ainda segura: a pilha de kernel do slot da tarefa, por
/// exemplo. Mede-se uma vez, com o `init`, depois de uma rodada de aquecimento (o heap já cresceu).
fn settled_slack(frames: Frames, map_offset: u64, init: &[u8]) -> Option<usize> {
    run_source(frames, map_offset, Source::Elf(init), here())?;
    let before = frames.free_frames();
    run_source(frames, map_offset, Source::Elf(init), here())?;
    Some(before.saturating_sub(frames.free_frames()))
}

/// O `big` do initramfs (mais de 2 MiB, com `.rodata`, `.data` e `.bss` de muitas páginas) roda e sai
/// com 0, e os frames voltam exatamente como depois do `init`: o carregador copia, zera e mapeia
/// segmentos grandes, o `-C code-model=small` aguenta, e nada vaza.
fn big_program_runs(frames: Frames, map_offset: u64, archive: Option<&[u8]>) -> bool {
    let find = |name: &[u8]| archive.and_then(|archive| loader::find(archive, name).ok());
    let (Some(init), Some(big)) = (find(b"init"), find(b"big")) else {
        return false;
    };
    let Some(slack) = settled_slack(frames, map_offset, init) else {
        return false;
    };
    let before = frames.free_frames();
    let result = run_source(frames, map_offset, Source::Elf(big), here());
    let kept = before.saturating_sub(frames.free_frames());
    kprintln!(
        "[rustos] user: big ran ({} KiB of file), {kept} frames kept after the exit, {slack} after the init",
        big.len() >> 10
    );
    exited_with(result, 0) && kept == slack
}

/// Espera a mensagem `want` na ponta do kernel do canal de boot, até 10 s.
fn wait_for_message(peer: &super::boot::BootPeer, want: &[u8]) -> bool {
    let Some(start) = platform::now_ns() else {
        return false;
    };
    while platform::now_ns().is_some_and(|now| now < start + WAIT_NS) {
        if peer.read().is_ok_and(|got| got == want) {
            return true;
        }
        task::sleep_until(platform::now_ns().unwrap_or(0) + POLL_NS);
    }
    false
}

/// O programa `handles` roda os casos hostis e sai com 0. No fim, com a ponta do canal de boot na
/// mão do kernel, o programa bloqueia em `object_wait_one` e só volta quando o kernel escreve e
/// depois fecha a ponta: a espera acorda por sinal, não pelo prazo de 10 s.
fn handles_program_runs(frames: Frames, map_offset: u64, archive: Option<&[u8]>) -> bool {
    let Some(file) = archive.and_then(|archive| loader::find(archive, b"handles").ok()) else {
        return false;
    };
    let Some(started) = platform::now_ns() else {
        return false;
    };
    let Ok((spawned, Some(peer))) = spawn_keeping(frames, map_offset, Source::Elf(file), 0, true)
    else {
        return false;
    };
    let wake = |message: &[u8], then: &dyn Fn()| {
        let ready = wait_for_message(&peer, message);
        task::sleep_until(platform::now_ns().unwrap_or(0) + 20_000_000);
        then();
        ready
    };
    let first = wake(b"ready", &|| {
        let _ = peer.write(b"go");
    });
    let second = wake(b"ready2", &|| {});
    peer.close();
    let result = finish(&spawned);
    let seconds = platform::now_ns().unwrap_or(0).saturating_sub(started) / 1_000_000_000;
    exited_with(result, 0) && first && second && seconds < 5
}

/// O `lazy` do initramfs: pilha e `.bss` sob demanda, com os números por página (4c).
fn lazy_program_runs(frames: Frames, map_offset: u64, archive: Option<&[u8]>) -> bool {
    let find = |name: &[u8]| archive.and_then(|archive| loader::find(archive, name).ok());
    let (Some(init), Some(lazy)) = (find(b"init"), find(b"lazy")) else {
        return false;
    };
    let Some(slack) = settled_slack(frames, map_offset, init) else {
        return false;
    };
    demand::lazy_program(frames, map_offset, lazy, slack)
}

/// Pixel (0, 0) que o `fb` pinta: azul 0, verde 0, vermelho cheio, em XRGB8888 (`0x00FF_0000`).
const FB_PATTERN_ORIGIN: u32 = 0x00FF_0000;

/// O `fb` do initramfs: mapeia a tela como VMO e pinta um padrão. Enquanto ele a segura, o console não
/// desenha (conta as vezes que quis, e o padrão segue na tela); quando ele sai, o console redesenha e
/// o pixel (0, 0) volta à barra (SDD 18, caso 6).
fn fb_program_runs(frames: Frames, map_offset: u64, archive: Option<&[u8]>) -> bool {
    let Some(file) = archive.and_then(|archive| loader::find(archive, b"fb").ok()) else {
        return false;
    };
    let (Some((width, height, pitch)), Some(before)) =
        (framebuffer::size(), framebuffer::pixel(0, 0))
    else {
        return false;
    };
    let options = Options {
        keep_peer: true,
        initramfs: false,
        framebuffer: true,
    };
    let Ok((spawned, Some(peer))) =
        spawn_with(frames, map_offset, Source::Elf(file), 0, options, "fb", &[])
    else {
        return false;
    };
    let mapped = wait_for_message(&peer, b"mapped");
    let owned = framebuffer::held();
    let skipped_before = framebuffer::skipped();
    kprintln!("[rustos] fb self-test: the program holds the screen, the console must stay quiet");
    let paused = owned
        && framebuffer::skipped() > skipped_before
        && framebuffer::pixel(0, 0) == Some(FB_PATTERN_ORIGIN);
    let _ = peer.write(b"go");
    let exited = exited_with(finish(&spawned), 0);
    peer.close();
    let released = !framebuffer::held();
    console::flush();
    let restored = framebuffer::pixel(0, 0) == Some(before);
    let ok = mapped && paused && exited && released && restored;
    if ok {
        kprintln!(
            "[rustos] fb self-test: ok ({width}x{height} stride {pitch}, console paused and restored)"
        );
    } else {
        kprintln!(
            "[rustos] fb self-test: FAILED (mapped {mapped}, paused {paused}, exited {exited}, released {released}, restored {restored})"
        );
    }
    ok
}

/// Escreve `value` em `bytes` bytes little-endian a partir de `at`.
fn put(file: &mut [u8], at: usize, value: u64, bytes: usize) {
    file[at..at + bytes].copy_from_slice(&value.to_le_bytes()[..bytes]);
}

/// Um ELF de dois `PT_LOAD` com o segundo desalinhado da página (`p_vaddr = 0x40_3010`, `p_offset =
/// 0x2010`, `p_filesz = 0x20`, `p_memsz = 0x2020`), em arquivo cheio de `0xAA`: o que o carregador
/// copiar além dos 32 bytes aparece na memória do processo.
fn unaligned_elf() -> alloc::vec::Vec<u8> {
    let mut file = alloc::vec![0xAA_u8; 0x2030];
    file[..64 + 2 * 56].fill(0);
    file[..4].copy_from_slice(b"\x7fELF");
    file[4..7].copy_from_slice(&[2, 1, 1]);
    put(&mut file, 16, 2, 2);
    put(&mut file, 18, u64::from(loader::MACHINE), 2);
    put(&mut file, 20, 1, 4);
    put(&mut file, 24, 0x40_0100, 8);
    put(&mut file, 32, 64, 8);
    put(&mut file, 52, 64, 2);
    put(&mut file, 54, 56, 2);
    put(&mut file, 56, 2, 2);
    for (index, (flags, offset, vaddr, filesz, memsz)) in [
        (5, 0, 0x40_0000, 0x1000, 0x1000),
        (6, 0x2010, 0x40_3010, 0x20, 0x2020),
    ]
    .into_iter()
    .enumerate()
    {
        let at = 64 + index * 56;
        put(&mut file, at, 1, 4);
        put(&mut file, at + 4, flags, 4);
        for (field, value) in [
            (8, offset),
            (16, vaddr),
            (24, vaddr),
            (32, filesz),
            (40, memsz),
            (48, 0x1000),
        ] {
            put(&mut file, at + field, value, 8);
        }
    }
    file
}

/// Todo byte de `[virt, virt + len)` do espaço vale `expected(posição)`.
fn user_bytes_are(
    space: &UserSpace,
    virt: u64,
    len: usize,
    expected: impl Fn(usize) -> u8,
) -> bool {
    let mut chunk = [0_u8; 256];
    (0..len).step_by(256).all(|start| {
        let take = 256.min(len - start);
        let part = &mut chunk[..take];
        space.read(virt + start as u64, part)
            && part
                .iter()
                .enumerate()
                .all(|(index, &byte)| byte == expected(start + index))
    })
}

/// O caminho `lead > 0` do carregador: o segmento começa a `0x10` bytes da página, e o começo dela e a
/// cauda `.bss` até `p_memsz` vêm zerados; só os 32 bytes do arquivo aparecem. Lê pelos frames do
/// espaço, sem rodar o programa.
fn unaligned_segment_loads_zeroed(frames: Frames, map_offset: u64) -> bool {
    let file = unaligned_elf();
    // SAFETY: o mapa direto cobre todo frame do alocador e esta tarefa roda nas tabelas do kernel.
    let Ok(mut space) = (unsafe { UserSpace::new(frames, map_offset) }) else {
        return false;
    };
    let Ok(mut tree) = loader::new_tree() else {
        space.destroy();
        return false;
    };
    let loaded = loader::load_image(&mut space, &mut tree, &file);
    let lazy_tail = [0x40_4000, 0x40_5000]
        .iter()
        .all(|&page| !space.read(page, &mut [0; 1]));
    let resolved = [0x40_4000, 0x40_5000]
        .iter()
        .all(|&page| space.resolve_fault(&tree, page, Access::Read) == Ok(Resolved::ZeroPage));
    let ok = loaded.is_ok_and(|loaded| loaded.segments == 2)
        && lazy_tail
        && resolved
        && user_bytes_are(&space, 0x40_0000, 4, |index| b"\x7fELF"[index])
        && user_bytes_are(&space, 0x40_3000, 0x1000, |index| {
            if (0x10..0x30).contains(&index) {
                0xAA
            } else {
                0
            }
        })
        && user_bytes_are(&space, 0x40_4000, 0x1000, |_| 0)
        && user_bytes_are(&space, 0x40_5000, 0x1000, |_| 0);
    space.destroy();
    ok
}

/// O carregador recusa o que o `core/elf` recusa (arquivo cortado na tabela de program headers,
/// máquina de outra ISA, sem magic, vazio) e, quando a falta de frame vem tarde, depois de segmentos
/// e de parte da pilha mapeados, devolve cada frame. Depois de uma rodada de aquecimento a conta é
/// exata.
fn loader_leaves_no_frame_behind(frames: Frames, map_offset: u64, archive: Option<&[u8]>) -> bool {
    let Some(good) = archive.and_then(|archive| loader::find(archive, b"init").ok()) else {
        return false;
    };
    let mut other_machine = good.to_vec();
    other_machine[18] ^= 0xFF;
    let mut bad_magic = good.to_vec();
    bad_magic[0] = 0;
    let try_spawn = |file: &[u8]| spawn(frames, map_offset, Source::Elf(file), 0).is_err();
    let early: [&[u8]; 4] = [&good[..100], &other_machine, &bad_magic, &[]];
    let _ = early.map(try_spawn);
    let before = frames.free_frames();
    let refused = early.into_iter().all(try_spawn);
    let early_ok = refused && frames.free_frames() == before;
    // A falta de frame na página `n` de dados: a primeira, uma do meio e a última, que é a da pilha
    // com os argumentos (a pilha só ganha essa; o resto dela é preguiçoso).
    let mut late_ok = true;
    let Some(total) = spawned_data_frames(frames, map_offset, good) else {
        return false;
    };
    for budget in [1, total / 2, total - 1] {
        let before = frames.free_frames();
        set_frame_budget(Some(budget));
        let refused = try_spawn(good);
        set_frame_budget(None);
        late_ok &= refused && frames.free_frames() == before;
    }
    early_ok && late_ok
}

/// Os frames de dados que o carregador mapeia para `file`: o processo roda até sair.
fn spawned_data_frames(frames: Frames, map_offset: u64, file: &[u8]) -> Option<u64> {
    let spawned = spawn(frames, map_offset, Source::Elf(file), here()).ok()?;
    let deadline = platform::now_ns()? + WAIT_NS;
    while !task::has_exited(spawned.task) {
        let now = platform::now_ns()?;
        if now > deadline {
            return None;
        }
        task::sleep_until(now + POLL_NS);
    }
    Some(spawned.stats.data)
}

pub(super) fn exited_with(result: Option<(u32, ProcessEvent)>, code: i64) -> bool {
    result.is_some_and(|(_, event)| event.kind == event_kind::EXIT && event.code == code)
}

fn faulted(result: Option<(u32, ProcessEvent)>) -> Option<ProcessEvent> {
    result
        .map(|(_, event)| event)
        .filter(|event| event.kind == event_kind::FAULT)
}

/// O `sp` que `fault-bad-stack` recebe pelo `argv` e põe no `rsp`/`sp`: o evento o leva só até `USER_TOP`.
#[cfg(target_arch = "x86_64")]
const BAD_STACK: u64 = 0x1234_8000_0000_0000;
#[cfg(target_arch = "aarch64")]
const BAD_STACK: u64 = 1;

/// O `fault_kind` e o `isa_code` que cada falta do `cases` deixa nesta ISA, medidos com o programa.
/// O `isa_code` é o vetor e o código de erro do x86_64 e a `ESR` do aarch64.
#[cfg(target_arch = "x86_64")]
fn expected_fault(case: &str) -> Option<(u32, u64)> {
    Some(match case {
        "fault-memory" => (fault_kind::PAGE_FAULT, 0xe_0000_0005),
        "fault-privileged" | "fault-bad-stack" => (fault_kind::OTHER, 0xd_0000_0000),
        "fault-trap" => (fault_kind::OTHER, 0x1_0000_0000),
        _ => return None,
    })
}
#[cfg(target_arch = "aarch64")]
fn expected_fault(case: &str) -> Option<(u32, u64)> {
    Some(match case {
        "fault-memory" => (fault_kind::PAGE_FAULT, 0x9200_000f),
        "fault-privileged" => (fault_kind::ILLEGAL_INSTRUCTION, 0x200_0000),
        "fault-trap" => (fault_kind::BREAKPOINT, 0xf200_0000),
        "fault-bad-stack" => (fault_kind::UNALIGNED, 0x9a00_0000),
        _ => return None,
    })
}

/// A falta que `case` deve causar nesta ISA: o evento de morte por falta com o `fault_kind` e o
/// `isa_code` esperados. Qualquer outra falta, ou saída, dá `None`.
fn caused(result: Option<(u32, ProcessEvent)>, case: &str) -> Option<ProcessEvent> {
    let (kind, isa) = expected_fault(case)?;
    faulted(result).filter(|event| {
        (event.fault_kind == kind && event.isa_code == isa) || unchecked_alignment(case, event)
    })
}

/// O TCG da QEMU 8.2 (o do CI) não recusa o `sp` desalinhado do `fault-bad-stack`, como o HVF faz:
/// o `ldr` vira falta de página no próprio `sp`. Os dois provam o caso.
#[cfg(target_arch = "aarch64")]
fn unchecked_alignment(case: &str, event: &ProcessEvent) -> bool {
    case == "fault-bad-stack"
        && event.fault_kind == fault_kind::PAGE_FAULT
        && event.addr == BAD_STACK
}

#[cfg(not(target_arch = "aarch64"))]
fn unchecked_alignment(_: &str, _: &ProcessEvent) -> bool {
    false
}

/// O endereço e o `sp` como o evento os leva: `ProcessEvent::for_user` zera o que está fora de
/// `[0, USER_TOP)`, então um endereço do kernel nunca aparece no evento.
fn visible_addr(addr: u64) -> u64 {
    if addr < USER_TOP { addr } else { 0 }
}
fn visible_sp(sp: u64) -> u64 {
    if sp <= USER_TOP { sp } else { 0 }
}

/// Décimos de ns por chamada de uma execução do laço, na CPU `cpu` (sem migração no meio).
fn sample_tenths_ns(
    frames: Frames,
    map_offset: u64,
    archive: Option<&[u8]>,
    cpu: usize,
) -> Option<u64> {
    let (_, event) = run_case(
        frames,
        map_offset,
        archive,
        "null-loop",
        LOOP_CALLS,
        1 << cpu,
    )?;
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

fn null_loop_medians(frames: Frames, map_offset: u64, archive: Option<&[u8]>) -> Option<Medians> {
    let cpu = usize::from(task::active_cpus() & 2 != 0);
    let (mut clean, mut dirty, mut slow) = ([0; RUNS], [0; RUNS], [0; RUNS]);
    for round in 0..RUNS {
        scrub::set_enabled(true);
        clean[round] = sample_tenths_ns(frames, map_offset, archive, cpu)?;
        scrub::set_enabled(false);
        dirty[round] = sample_tenths_ns(frames, map_offset, archive, cpu)?;
        scrub::set_enabled(true);
        if platform::HAS_FAST_RETURN {
            platform::set_fast_return(false);
            let sample = sample_tenths_ns(frames, map_offset, archive, cpu);
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
fn residue(frames: Frames, map_offset: u64, archive: Option<&[u8]>) -> Option<usize> {
    FOUND.store(usize::MAX, Ordering::Relaxed);
    PROBE.store(PLANT_NEXT, Ordering::Relaxed);
    run_case(frames, map_offset, archive, "null-loop", 16, 0)?;
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
fn hello_on_every_cpu(frames: Frames, map_offset: u64, archive: Option<&[u8]>) -> bool {
    let active = task::active_cpus();
    (0..rustos_hal::MAX_CPUS)
        .filter(|cpu| active >> cpu & 1 != 0)
        .all(|cpu| {
            let result = run_case(frames, map_offset, archive, "hello", 0, 1 << cpu);
            exited_with(result, 0)
        })
}

/// Os casos do programa embutido. `false` se algum falhou.
#[allow(
    clippy::too_many_lines,
    reason = "a lista linear dos casos do autoteste"
)]
pub fn self_test(frames: Frames, map_offset: u64, archive: Option<&[u8]>) -> bool {
    let hello = hello_on_every_cpu(frames, map_offset, archive) && copy_fixup_ran_with_ac_clear();
    let probe = ptr::addr_of!(KERNEL_PROBE).expose_provenance() as u64;
    let kernel = caused(
        run_case(frames, map_offset, archive, "fault-memory", probe, 0),
        "fault-memory",
    );
    let kernel_ok = kernel.is_some_and(|event| {
        event.access == fault_access::PROTECTION && event.addr == visible_addr(probe)
    });
    AC_LEAKS.store(0, Ordering::Relaxed);
    let privileged = caused(
        run_case(frames, map_offset, archive, "fault-privileged", 0, 0),
        "fault-privileged",
    )
    .is_some()
        && AC_LEAKS.load(Ordering::Relaxed) == 0;
    let trap = caused(
        run_case(frames, map_offset, archive, "fault-trap", 0, 0),
        "fault-trap",
    )
    .is_some();
    let stack = caused(
        run_case(frames, map_offset, archive, "fault-bad-stack", BAD_STACK, 0),
        "fault-bad-stack",
    )
    .is_some_and(|event| event.sp == visible_sp(BAD_STACK));
    let flags = flags_survive(frames, map_offset, archive);
    let resolver = demand::resolver_cases(frames, map_offset);
    let big = big_program_runs(frames, map_offset, archive);
    let handles = handles_program_runs(frames, map_offset, archive);
    let processes = procs::run(frames, map_offset, archive);
    let refused = loader_leaves_no_frame_behind(frames, map_offset, archive);
    let unaligned = unaligned_segment_loads_zeroed(frames, map_offset);
    let lazy = resolver && lazy_program_runs(frames, map_offset, archive);
    let screen = fb_program_runs(frames, map_offset, archive);
    let shot = demand::shootdown(frames, map_offset)
        && demand::ring_stress(frames, map_offset, demand::RING_ROUNDS, false)
        && demand::ring_stress(frames, map_offset, demand::RING_ROUNDS, true);
    let tagged = tags::isolation(frames, map_offset);
    let switch = tags::switch_ns(frames, map_offset);
    let (syscall_depth, fault_depth) = (scrub::deepest(), fault_peak());
    scrub::set_measuring(false);
    let clean_found = residue(frames, map_offset, archive);
    scrub::set_enabled(false);
    let dirty_found = residue(frames, map_offset, archive);
    scrub::set_enabled(true);
    let medians = null_loop_medians(frames, map_offset, archive);
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
    if let Some(ns) = switch {
        kprintln!(
            "[rustos] asid bench: {ns} ns per switch between two spaces, one read each ({} bits)",
            asid::bits()
        );
    }
    let ok = hello
        && kernel_ok
        && privileged
        && trap
        && stack
        && flags
        && scrub_ok
        && big
        && handles
        && processes
        && refused
        && unaligned
        && resolver
        && lazy
        && screen
        && shot
        && tagged
        && switch.is_some()
        && medians.is_some();
    kprintln!(
        "[rustos] user self-test: {} (hello {hello}, kernel page {kernel_ok}, privileged {privileged}, trap {trap}, bad stack {stack}, flags {flags}, stack clean {scrub_ok}, big {big}, handles {handles}, processes {processes}, bad executables {refused}, unaligned segment {unaligned}, fault resolver {resolver}, lazy program {lazy}, tlb shootdown {shot})",
        if ok { "ok" } else { "FAILED" }
    );
    ok
}
