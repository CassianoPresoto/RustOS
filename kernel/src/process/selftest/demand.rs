//! Paginação sob demanda e *shootdown* de TLB no modo de teste (SDD 17, fatia 4c): o resolvedor da
//! falta contra um espaço montado à mão, o programa `lazy` do initramfs, a reserva de páginas e o
//! teste entre CPUs em que uma remove o que a outra lê.

use alloc::boxed::Box;
use core::hint::spin_loop;
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use rustos_hal::{Cpu, PAGE_SIZE, PageFlags};
use rustos_mm::UserRange;
use rustos_mm::vmar::{Access, Attrs, FaultError, Kind, Perms};
use rustos_sched::SchedParams;

use super::{exited_with, here, run_source};
use crate::frames::Frames;
use crate::process::{Source, last_stats};
use crate::task::{self, Spawn};
use crate::uspace::{
    Pages, Resolved, Unresolved, UserSpace, fault_counts, reset_fault_counts, zero_page_frame,
    zero_page_is_clean,
};
use crate::{commit, loader, platform, tlb};

const RW: Perms = Perms::READ.union(Perms::WRITE);
const PATTERN: u64 = 0xC3C3_C3C3_C3C3_C3C3;
const OLD_PATTERN: u64 = 0x5A5A_5A5A_5A5A_5A5A;
const PAGE: u64 = PAGE_SIZE;

/// O espaço, a árvore e a página que o resolvedor lê: uma região preguiçosa de 8 páginas com guardas,
/// uma página de imagem só de leitura, já mapeada.
fn build(frames: Frames, map_offset: u64) -> Option<(UserSpace, loader::RegionTree)> {
    // SAFETY: o mapa direto cobre todo frame do alocador e esta tarefa roda nas tabelas do kernel.
    let mut space = unsafe { UserSpace::new(frames, map_offset) }.ok()?;
    let mut tree = loader::new_tree().ok()?;
    let stack = Attrs::new(Kind::Stack, RW).with_guards(true, true).lazy();
    let image = Attrs::new(Kind::Image, Perms::READ);
    let ok = tree.reserve(LAZY, 8 * PAGE, stack).is_ok()
        && space.reserve_lazy(8).is_ok()
        && tree.reserve(IMAGE, PAGE, image).is_ok()
        && space
            .map_at(IMAGE, 0, &[1, 2, 3], 1, PageFlags::USER_RODATA)
            .is_ok();
    if !ok {
        space.destroy();
        return None;
    }
    Some((space, tree))
}

const LAZY: u64 = 0x1000_0000;
const IMAGE: u64 = 0x2000_0000;

fn all_zero(space: &UserSpace, virt: u64) -> bool {
    let mut page = [0xFF_u8; 256];
    (0..PAGE)
        .step_by(256)
        .all(|offset| space.read(virt + offset, &mut page) && page.iter().all(|&byte| byte == 0))
}

/// Leitura, escrita e escrita sobre a página zero numa região preguiçosa, cada caso de morte e a
/// contabilidade: o que o espaço devolve ao morrer é tudo o que pegou, a página zero fica.
fn resolver_walks_every_case(frames: Frames, map_offset: u64) -> bool {
    let Some((mut space, tree)) = build(frames, map_offset) else {
        return false;
    };
    let fault = |space: &mut UserSpace, at: u64, access| space.resolve_fault(&tree, at, access);
    let first_read = fault(&mut space, LAZY + 3 * PAGE + 8, Access::Read);
    let zero = zero_page_frame();
    let shared = space.read(LAZY + 3 * PAGE, &mut [1; 8]) && all_zero(&space, LAZY + 3 * PAGE);
    let mut ok = first_read == Ok(Resolved::ZeroPage) && zero.is_some() && shared;
    ok &= fault(&mut space, LAZY + 3 * PAGE, Access::Read) == Ok(Resolved::Retried);
    ok &= fault(&mut space, LAZY + 3 * PAGE, Access::Write) == Ok(Resolved::CopiedZero);
    ok &= all_zero(&space, LAZY + 3 * PAGE);
    ok &= fault(&mut space, LAZY + 5 * PAGE, Access::Write) == Ok(Resolved::Fresh);
    ok &= fault(&mut space, LAZY + 5 * PAGE, Access::Write) == Ok(Resolved::Retried);
    ok &= fault(&mut space, LAZY, Access::Read) == Ok(Resolved::ZeroPage)
        && fault(&mut space, LAZY + PAGE, Access::Read) == Ok(Resolved::ZeroPage);
    ok &= space.stats().lazy_backed == 2;
    ok &=
        fault(&mut space, LAZY - PAGE, Access::Write) == Err(Unresolved::Region(FaultError::Guard));
    ok &= fault(&mut space, LAZY + 8 * PAGE, Access::Read)
        == Err(Unresolved::Region(FaultError::Guard));
    ok &= fault(&mut space, 0x3000_0000, Access::Read)
        == Err(Unresolved::Region(FaultError::NoRegion));
    ok &=
        fault(&mut space, IMAGE, Access::Write) == Err(Unresolved::Region(FaultError::Permission));
    ok &= fault(&mut space, LAZY + 2 * PAGE, Access::Execute)
        == Err(Unresolved::Region(FaultError::Permission));
    ok &= fault(&mut space, IMAGE, Access::Read) == Ok(Resolved::Retried);
    ok &= space.zero_page_is_read_only_everywhere();
    let promised = commit::reserved();
    space.destroy();
    ok && promised >= 8 - 2 && zero == zero_page_frame() && zero_page_is_clean()
}

/// Passa o resolvedor duas vezes (a primeira cria a página zero e aquece o heap) e confere que a
/// segunda devolve exatamente os frames e a reserva que pegou.
pub fn resolver_cases(frames: Frames, map_offset: u64) -> bool {
    let warm = resolver_walks_every_case(frames, map_offset);
    let (free, reserved) = (frames.free_frames(), commit::reserved());
    let measured = resolver_walks_every_case(frames, map_offset);
    let balanced = frames.free_frames() == free && commit::reserved() == reserved;
    // SAFETY: como em `build`.
    let greedy = unsafe { UserSpace::new(frames, map_offset) }.is_ok_and(|mut space| {
        let refused = space.reserve_lazy(u64::MAX / 2).is_err();
        space.destroy();
        refused
    });
    let kept = commit::reserved() == reserved;
    let tables = promise_covers_the_tables(frames, map_offset);
    warm && measured && balanced && greedy && kept && tables
}

/// A promessa inclui as tabelas de página: pedir tantas páginas quantos são os frames livres falha,
/// e pedir o que sobra depois das tabelas passa.
fn promise_covers_the_tables(frames: Frames, map_offset: u64) -> bool {
    // SAFETY: como em `build`.
    let Ok(mut space) = (unsafe { UserSpace::new(frames, map_offset) }) else {
        return false;
    };
    let free = frames.free_frames() as u64;
    let too_many = space.reserve_lazy(free).is_err();
    let fits = space.reserve_lazy(free - free.div_ceil(512) - 4).is_ok();
    space.destroy();
    too_many && fits
}

/// O `lazy` do initramfs toca a pilha e um `.bss` grande, lê antes de escrever e confere zero, e
/// mede por página os três caminhos da falta. Os frames voltam como depois do `init`.
pub fn lazy_program(frames: Frames, map_offset: u64, lazy: &[u8], slack: usize) -> bool {
    reset_fault_counts();
    let before = frames.free_frames();
    let reserved = commit::reserved();
    let result = run_source(frames, map_offset, Source::Elf(lazy), here());
    let kept = before.saturating_sub(frames.free_frames());
    let [zero, fresh, copied, retried] = fault_counts();
    let backed = last_stats().map_or((0, 0), |(_, stats)| {
        (stats.lazy_backed, stats.lazy_reserved)
    });
    kprintln!(
        "[rustos] user: demand paging: {zero} zero-page maps, {fresh} fresh frames, {copied} zero-page copies, {retried} retries; {} of {} lazy pages backed at exit, {kept} frames kept (slack {slack})",
        backed.0,
        backed.1
    );
    exited_with(result, 0)
        && zero >= PROGRAM_PAGES - EDGE
        && fresh >= PROGRAM_PAGES
        && copied >= PROGRAM_PAGES - EDGE
        && backed.0 >= 2 * PROGRAM_PAGES + STACK_TOUCHED - EDGE
        && kept == slack
        && commit::reserved() == reserved
}

/// Páginas de cada laço do `lazy` (`user/tests/lazy`): os três caminhos passam por tantas.
const PROGRAM_PAGES: u64 = 4096;
/// Páginas que um laço do `lazy` pode não fazer faltar: a da ponta do `.bss` que divide a página com o
/// `.data` já vem com frame.
const EDGE: u64 = 2;
/// Páginas da pilha que o `lazy` desce além da dos argumentos.
const STACK_TOUCHED: u64 = 8;

/// Uma rodada do teste entre CPUs: a CPU `watch` roda uma tarefa com o espaço carregado e lê duas
/// páginas; a CPU `drive` troca uma por um frame novo (a cópia da página zero) e remove a outra.
struct Pass {
    frames: Frames,
    map_offset: u64,
    remote: bool,
    bench: bool,
    watch: usize,
    stage: AtomicU32,
    root: AtomicU64,
    active: AtomicUsize,
    cow_seen: AtomicU64,
    gone_seen: AtomicU64,
    ok: AtomicBool,
    flush_tenths_ns: AtomicU64,
}

const UNREADABLE: u64 = u64::MAX;
const FLUSHES: u64 = if cfg!(target_arch = "x86_64") {
    200
} else {
    2000
};

fn read_word(at: u64) -> u64 {
    let mut bytes = [0_u8; 8];
    match UserRange::new(at, 8) {
        Ok(range) if platform::copy_from_user(range, &mut bytes) => u64::from_le_bytes(bytes),
        _ => UNREADABLE,
    }
}

fn wait_stage(pass: &Pass, stage: u32) {
    while pass.stage.load(Ordering::Acquire) != stage {
        spin_loop();
    }
}

/// A tarefa da CPU que lê: carrega o espaço do outro, lê, espera a troca e lê de novo.
fn watcher(arg: usize) {
    // SAFETY: `arg` é o endereço do `Pass` na pilha de quem espera esta tarefa sair.
    let pass = unsafe { &*core::ptr::with_exposed_provenance::<Pass>(arg) };
    let cpu = platform::cpu();
    task::bind_user(
        pass.root.load(Ordering::Acquire),
        0,
        pass.active.load(Ordering::Acquire),
    );
    cpu.restore_interrupts(true);
    let first = (read_word(LAZY), read_word(LAZY + PAGE));
    pass.ok.store(first == (0, OLD_PATTERN), Ordering::Relaxed);
    pass.stage.store(1, Ordering::Release);
    wait_stage(pass, 2);
    pass.cow_seen.store(read_word(LAZY), Ordering::Relaxed);
    pass.gone_seen
        .store(read_word(LAZY + PAGE), Ordering::Relaxed);
    pass.stage.store(3, Ordering::Release);
    cpu.save_and_disable_interrupts();
    let _ = task::unbind_user();
    task::exit()
}

/// A tarefa da outra CPU: monta o espaço, muda o que a primeira lê e espera ela terminar.
fn driver(arg: usize) {
    // SAFETY: como em `watcher`.
    let pass = unsafe { &*core::ptr::with_exposed_provenance::<Pass>(arg) };
    let Some((mut space, tree)) = build_shared(pass) else {
        pass.ok.store(false, Ordering::Relaxed);
        pass.stage.store(3, Ordering::Release);
        return;
    };
    let space_ref = &mut *space;
    pass.root
        .store(space_ref.root().as_u64(), Ordering::Release);
    pass.active
        .store(space_ref.active_addr(), Ordering::Release);
    let spawned = task::spawn(Spawn {
        name: "watch",
        entry: watcher,
        arg,
        simd: false,
        params: SchedParams::fair(0),
        affinity: 1 << pass.watch,
    });
    if spawned.is_err() {
        pass.stage.store(3, Ordering::Release);
    }
    wait_stage(pass, 1);
    let watched = space_ref.active_cpus() >> pass.watch & 1 != 0;
    if pass.bench {
        measure(pass, space_ref);
    }
    tlb::set_remote(pass.remote);
    let copied = space_ref.resolve_fault(&tree, LAZY, Access::Write) == Ok(Resolved::CopiedZero)
        && space_ref.write_private(LAZY, &PATTERN.to_le_bytes());
    let removed = space_ref.unmap_page(LAZY + PAGE, Pages::Owned);
    tlb::set_remote(true);
    pass.ok
        .fetch_and(watched && copied && removed, Ordering::Relaxed);
    pass.stage.store(2, Ordering::Release);
    wait_stage(pass, 3);
    space.wait_inactive();
    space.destroy();
}

/// O espaço da rodada: uma página preguiçosa já lida (a página zero) e uma própria com `OLD_PATTERN`.
fn build_shared(pass: &Pass) -> Option<(Box<UserSpace>, loader::RegionTree)> {
    // SAFETY: o mapa direto cobre todo frame do alocador e esta tarefa roda nas tabelas do kernel.
    let mut space = Box::new(unsafe { UserSpace::new(pass.frames, pass.map_offset) }.ok()?);
    let mut tree = loader::new_tree().ok()?;
    let lazy = Attrs::new(Kind::Mapping, RW).lazy();
    let eager = Attrs::new(Kind::Mapping, RW);
    let ok = tree.reserve(LAZY, PAGE, lazy).is_ok()
        && space.reserve_lazy(1).is_ok()
        && tree.reserve(LAZY + PAGE, PAGE, eager).is_ok()
        && space
            .map_at(
                LAZY + PAGE,
                0,
                &OLD_PATTERN.to_le_bytes(),
                1,
                PageFlags::USER_DATA,
            )
            .is_ok()
        && space.resolve_fault(&tree, LAZY, Access::Read) == Ok(Resolved::ZeroPage);
    if !ok {
        space.destroy();
        return None;
    }
    Some((space, tree))
}

/// Custo de invalidar uma página nas outras CPUs que rodam o espaço (o IPI de ida e volta no
/// x86_64; o `tlbi ...is` no aarch64), em décimos de ns.
fn measure(pass: &Pass, space: &UserSpace) {
    let start = platform::cycles();
    for _ in 0..FLUSHES {
        space.flush_page(LAZY);
    }
    let ns = platform::cycles_to_ns(platform::cycles().saturating_sub(start));
    pass.flush_tenths_ns
        .store(ns * 10 / FLUSHES, Ordering::Relaxed);
}

/// Roda uma rodada com `watch` lendo e `drive` mudando; devolve o que a leitora viu depois.
fn run_pass(frames: Frames, map_offset: u64, cpus: (usize, usize), remote: bool) -> Option<Pass> {
    let pass = Pass {
        frames,
        map_offset,
        remote,
        bench: remote,
        watch: cpus.0,
        stage: AtomicU32::new(0),
        root: AtomicU64::new(0),
        active: AtomicUsize::new(0),
        cow_seen: AtomicU64::new(u64::MAX - 1),
        gone_seen: AtomicU64::new(u64::MAX - 1),
        ok: AtomicBool::new(true),
        flush_tenths_ns: AtomicU64::new(0),
    };
    let arg = core::ptr::from_ref(&pass).expose_provenance();
    let id = task::spawn(Spawn {
        name: "drive",
        entry: driver,
        arg,
        simd: false,
        params: SchedParams::fair(0),
        affinity: 1 << cpus.1,
    })
    .ok()?;
    let deadline = platform::now_ns()? + 30_000_000_000;
    while !task::has_exited(id) || pass.stage.load(Ordering::Acquire) != 3 {
        let now = platform::now_ns()?;
        if now > deadline {
            return None;
        }
        task::sleep_until(now + 1_000_000);
    }
    Some(pass)
}

/// Um espaço lido por uma CPU e mudado por outra: depois do COW da página zero e da remoção de uma
/// página, a leitora vê o frame novo e a falta, nunca a tradução velha. Sem a invalidação remota a
/// tradução velha sobrevive (o controle, que só se conta). Também mede o custo da invalidação.
pub fn shootdown(frames: Frames, map_offset: u64) -> bool {
    let active = task::active_cpus();
    let mut cpus = (0..rustos_hal::MAX_CPUS).filter(|cpu| active >> cpu & 1 != 0);
    let (Some(watch), Some(drive)) = (cpus.next_back(), cpus.next_back()) else {
        kprintln!("[rustos] tlb shootdown: skipped, one cpu");
        return true;
    };
    let Some(good) = run_pass(frames, map_offset, (watch, drive), true) else {
        return false;
    };
    let seen = (
        good.cow_seen.load(Ordering::Relaxed),
        good.gone_seen.load(Ordering::Relaxed),
    );
    let proper = seen == (PATTERN, UNREADABLE) && good.ok.load(Ordering::Relaxed);
    let stale = run_pass(frames, map_offset, (watch, drive), false).is_some_and(|control| {
        control.cow_seen.load(Ordering::Relaxed) == 0
            && control.gone_seen.load(Ordering::Relaxed) != UNREADABLE
    });
    let flush = good.flush_tenths_ns.load(Ordering::Relaxed);
    kprintln!(
        "[rustos] tlb shootdown: {} (cpu {drive} -> cpu {watch}: a copied zero page and an unmapped page are gone for the reader); without the remote flush the old translation survives: {}; {}.{} ns per page flush ({FLUSHES} flushes)",
        if proper { "ok" } else { "FAILED" },
        if stale { "yes" } else { "no" },
        flush / 10,
        flush % 10
    );
    proper
}

const RING_MAX: usize = rustos_hal::MAX_CPUS;
/// Rodadas de cada passo do anel no autoteste (cada uma faz os três passos).
pub const RING_ROUNDS: u64 = 300;

struct Ring {
    n: usize,
    count: u32,
    frames: Frames,
    map_offset: u64,
    iters: u64,
    masked: bool,
    boxes: [AtomicPtr<(UserSpace, loader::RegionTree)>; RING_MAX],
    roots: [AtomicU64; RING_MAX],
    actives: [AtomicUsize; RING_MAX],
    state: [AtomicU64; RING_MAX],
    ack: [AtomicU64; RING_MAX],
    built: AtomicU32,
    bound: AtomicU32,
    finished: AtomicU32,
    exited: AtomicU32,
    bad: AtomicU64,
    first_bad: AtomicU64,
    checks: AtomicU64,
}

struct RingArg {
    ring: *const Ring,
    index: usize,
}

fn ring_pattern(step: u64) -> u64 {
    0xC000_0000_0000_0000 | step
}

/// O espaço de uma CPU do anel: uma página preguiçosa, lida e depois copiada pelas outras.
fn ring_build(ring: &Ring) -> Option<Box<(UserSpace, loader::RegionTree)>> {
    // SAFETY: como em `build_shared`.
    let mut space = unsafe { UserSpace::new(ring.frames, ring.map_offset) }.ok()?;
    let mut tree = loader::new_tree().ok()?;
    let lazy = Attrs::new(Kind::Mapping, RW).lazy();
    (tree.reserve(LAZY, PAGE, lazy).is_ok() && space.reserve_lazy(1).is_ok()).then_some(())?;
    Some(Box::new((space, tree)))
}

/// A tarefa de uma CPU do anel.
fn ring_task(arg: usize) {
    // SAFETY: o `RingArg` vive na pilha de quem espera as tarefas saírem.
    let arg = unsafe { &*core::ptr::with_exposed_provenance::<RingArg>(arg) };
    // SAFETY: como acima, para o `Ring`.
    let ring = unsafe { &*arg.ring };
    let (me, n) = (arg.index, ring.n);
    let cpu = platform::cpu();
    let built = ring_build(ring);
    let Some(boxed) = built else {
        ring.bad.fetch_add(1_000_000, Ordering::Relaxed);
        ring.built.store(ring.count + 100, Ordering::Release);
        ring.bound.fetch_add(1, Ordering::Release);
        ring.finished.fetch_add(1, Ordering::Release);
        ring.exited.fetch_add(1, Ordering::Release);
        return;
    };
    let raw = Box::into_raw(boxed);
    // SAFETY: acabou de sair de `Box::into_raw`.
    let root = unsafe { (*raw).0.root().as_u64() };
    // SAFETY: idem.
    let active = unsafe { (*raw).0.active_addr() };
    ring.roots[me].store(root, Ordering::Release);
    ring.actives[me].store(active, Ordering::Release);
    ring.boxes[me].store(raw, Ordering::Release);
    ring.built.fetch_add(1, Ordering::AcqRel);
    while ring.built.load(Ordering::Acquire) < ring.count {
        spin_loop();
    }
    task::bind_user(root, 0, active);
    cpu.restore_interrupts(true);
    ring.bound.fetch_add(1, Ordering::AcqRel);
    while ring.bound.load(Ordering::Acquire) < ring.count {
        spin_loop();
    }
    let target = (me + 1) % n;
    let steps = 3 * ring.iters;
    let (mut drive, mut seen) = (1_u64, 0_u64);
    while drive <= steps || seen < steps {
        if drive <= steps && ring.ack[target].load(Ordering::Acquire) == drive - 1 {
            // SAFETY: o alvo vive até todas as tarefas terminarem; só esta tarefa dirige as tabelas dele.
            let (space, tree) = unsafe { &mut *ring.boxes[target].load(Ordering::Acquire) };
            let were = if ring.masked {
                cpu.save_and_disable_interrupts()
            } else {
                false
            };
            let ok = match (drive - 1) % 3 {
                0 => space.resolve_fault(tree, LAZY, Access::Read) == Ok(Resolved::ZeroPage),
                1 => {
                    space.reserve_lazy(1).is_ok()
                        && space.resolve_fault(tree, LAZY, Access::Write)
                            == Ok(Resolved::CopiedZero)
                        && space.write_private(LAZY, &ring_pattern(drive).to_le_bytes())
                }
                _ => space.unmap_page(LAZY, Pages::Owned),
            };
            if ring.masked {
                cpu.restore_interrupts(were);
            }
            if !ok {
                ring.bad.fetch_add(1_000, Ordering::Relaxed);
                ring.first_bad
                    .compare_exchange(0, 1000 + drive, Ordering::Relaxed, Ordering::Relaxed)
                    .ok();
            }
            ring.state[target].store(drive, Ordering::Release);
            drive += 1;
        }
        let published = ring.state[me].load(Ordering::Acquire);
        if published > seen {
            let value = read_word(LAZY);
            let want = match (published - 1) % 3 {
                0 => 0,
                1 => ring_pattern(published),
                _ => UNREADABLE,
            };
            ring.checks.fetch_add(1, Ordering::Relaxed);
            if value != want {
                ring.bad.fetch_add(1, Ordering::Relaxed);
                ring.first_bad
                    .compare_exchange(0, published, Ordering::Relaxed, Ordering::Relaxed)
                    .ok();
            }
            seen = published;
            ring.ack[me].store(published, Ordering::Release);
        }
        spin_loop();
    }
    cpu.save_and_disable_interrupts();
    let _ = task::unbind_user();
    ring.finished.fetch_add(1, Ordering::AcqRel);
    while ring.finished.load(Ordering::Acquire) < ring.count {
        tlb::service();
        spin_loop();
    }
    // SAFETY: o `Box::into_raw` de acima; todas as tarefas já saíram da raiz (`finished`).
    unsafe { (*raw).0.wait_inactive() };
    // SAFETY: idem; cada espaço é destruído só pelo dono.
    let boxed = unsafe { Box::from_raw(raw) };
    let (space, tree) = *boxed;
    space.wait_inactive();
    space.destroy();
    drop(tree);
    ring.exited.fetch_add(1, Ordering::AcqRel);
}

/// Anel de CPUs: a CPU i dirige o espaço da CPU i+1 (página zero, cópia e remoção, em laço) e vigia
/// o seu, que a CPU i-1 dirige. Cada leitura tem de ver o valor do passo; com `masked` as mudanças
/// rodam com a IRQ mascarada, como a falta de página.
pub fn ring_stress(frames: Frames, map_offset: u64, iters: u64, masked: bool) -> bool {
    let active = task::active_cpus();
    let cpus: alloc::vec::Vec<usize> = (0..RING_MAX).filter(|cpu| active >> cpu & 1 != 0).collect();
    let n = cpus.len();
    if n < 2 {
        kprintln!("[rustos] tlb shootdown ring: skipped, one cpu");
        return true;
    }
    let ring = Ring {
        n,
        count: u32::try_from(n).unwrap_or(u32::MAX),
        frames,
        map_offset,
        iters,
        masked,
        boxes: [const { AtomicPtr::new(core::ptr::null_mut()) }; RING_MAX],
        roots: [const { AtomicU64::new(0) }; RING_MAX],
        actives: [const { AtomicUsize::new(0) }; RING_MAX],
        state: [const { AtomicU64::new(0) }; RING_MAX],
        ack: [const { AtomicU64::new(0) }; RING_MAX],
        built: AtomicU32::new(0),
        bound: AtomicU32::new(0),
        finished: AtomicU32::new(0),
        exited: AtomicU32::new(0),
        bad: AtomicU64::new(0),
        first_bad: AtomicU64::new(0),
        checks: AtomicU64::new(0),
    };
    let args: alloc::vec::Vec<RingArg> = (0..n)
        .map(|index| RingArg {
            ring: &raw const ring,
            index,
        })
        .collect();
    let started = platform::now_ns().unwrap_or(0);
    for (index, &cpu) in cpus.iter().enumerate() {
        let spawned = task::spawn(Spawn {
            name: "ring",
            entry: ring_task,
            arg: core::ptr::from_ref(&args[index]).expose_provenance(),
            simd: false,
            params: SchedParams::fair(0),
            affinity: 1 << cpu,
        });
        if spawned.is_err() {
            kprintln!("[rustos] tlb shootdown ring: FAILED (spawn)");
            return false;
        }
    }
    let deadline = started + 120_000_000_000;
    while ring.exited.load(Ordering::Acquire) < ring.count {
        let now = platform::now_ns().unwrap_or(0);
        if now > deadline {
            kprintln!(
                "[rustos] tlb shootdown ring: FAILED (timeout, exited {} bad {})",
                ring.exited.load(Ordering::Relaxed),
                ring.bad.load(Ordering::Relaxed)
            );
            return false;
        }
        task::sleep_until(now + 1_000_000);
    }
    let took = platform::now_ns().unwrap_or(0) - started;
    let bad = ring.bad.load(Ordering::Relaxed);
    kprintln!(
        "[rustos] tlb shootdown ring: {} (cpus {n}, {iters} rounds, irq {}, {} reads checked, {bad} stale, first at step {}, {} ms)",
        if bad == 0 { "ok" } else { "FAILED" },
        if masked { "masked" } else { "open" },
        ring.checks.load(Ordering::Relaxed),
        ring.first_bad.load(Ordering::Relaxed),
        took / 1_000_000
    );
    bad == 0
}
