//! Milhares de operações aleatórias, com semente fixa, contra um modelo ingênuo: o modelo guarda
//! os handles num mapa, as filas num `VecDeque` e decide a morte de uma ponta pelo ponto fixo da
//! regra do SDD 17 (alcançável se há handle fora da fila dela).

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::sync::Arc;

use rustos_hal::abi::{CHANNEL_MAX_BYTES, CHANNEL_QUEUE_LEN, Handle, Rights, Status};

use super::support::{Locked, Stack, Tab, Table, TestObj, VecSlots, count, new_channel, write};
use crate::rights::parse;
use crate::{
    HandleTable, Limits, Lock, Object, ObjectKind, Side, channel_read, close, close_pending,
};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

type EntId = usize;

struct Ent {
    obj: TestObj,
    root: Rights,
    end: Option<(usize, Side)>,
}

struct Msg {
    bytes: Vec<u8>,
    handles: Vec<(EntId, Rights)>,
}

struct World {
    rng: Rng,
    max_handles: usize,
    tabs: Vec<Tab>,
    ents: Vec<Ent>,
    live: Vec<BTreeMap<u64, (EntId, Rights)>>,
    seen: Vec<HashSet<u64>>,
    issued: Vec<Vec<u64>>,
    chans: Vec<[EntId; 2]>,
    queues: Vec<[VecDeque<Msg>; 2]>,
    dead: Vec<[bool; 2]>,
    log: Vec<&'static str>,
    stats: Stats,
}

/// O que a corrida exercitou: um modelo que nunca recusa nem mata ponta não prova nada.
#[derive(Default)]
struct Stats {
    writes_ok: usize,
    writes_with_handles: usize,
    write_errors: usize,
    reads_ok: usize,
    reads_with_handles: usize,
    read_errors: usize,
}

fn same(a: &TestObj, b: &TestObj) -> bool {
    match (a.plain_ref(), b.plain_ref()) {
        (Some(x), Some(y)) => Arc::ptr_eq(x, y),
        (None, None) => Arc::ptr_eq(a.end_ref().unwrap(), b.end_ref().unwrap()),
        _ => false,
    }
}

fn to_handle(raw: u64) -> Handle {
    Handle::from_raw(raw).unwrap_or(Handle::INVALID)
}

impl World {
    fn new(seed: u64, max_handles: usize, max_generation: u64) -> Self {
        let limits = Limits {
            max_handles,
            max_generation,
        };
        let tabs = (0..3)
            .map(|_| Locked::new(HandleTable::with_limits(VecSlots::default(), limits)))
            .collect();
        Self {
            rng: Rng(seed),
            max_handles,
            tabs,
            ents: Vec::new(),
            live: vec![BTreeMap::new(); 3],
            seen: vec![HashSet::new(); 3],
            issued: vec![Vec::new(); 3],
            chans: Vec::new(),
            queues: Vec::new(),
            dead: Vec::new(),
            log: Vec::new(),
            stats: Stats::default(),
        }
    }

    fn issue(&mut self, t: usize, handle: Handle) {
        assert!(
            self.seen[t].insert(handle.raw()),
            "o valor {:#x} reapareceu na tabela {t} ({:?})",
            handle.raw(),
            self.log.last()
        );
        self.issued[t].push(handle.raw());
    }

    fn all_refs(&self) -> Vec<u32> {
        let mut refs = vec![0; self.ents.len()];
        for (ent, _) in self.live.iter().flat_map(BTreeMap::values) {
            refs[*ent] += 1;
        }
        for queue in self.queues.iter().flatten() {
            for (ent, _) in queue.iter().flat_map(|msg| &msg.handles) {
                refs[*ent] += 1;
            }
        }
        refs
    }

    fn own(&self, c: usize, side: Side) -> u32 {
        let end = self.chans[c][side.index()];
        self.queues[c][side.index()]
            .iter()
            .flat_map(|msg| &msg.handles)
            .filter(|(e, _)| *e == end)
            .count() as u32
    }

    /// A regra do SDD, pelo ponto fixo: a ponta viva morre se todo handle dela está na fila dela.
    fn settle(&mut self) {
        loop {
            let mut changed = false;
            let refs = self.all_refs();
            for c in 0..self.chans.len() {
                for side in [Side::A, Side::B] {
                    if self.dead[c][side.index()] {
                        continue;
                    }
                    let end = self.chans[c][side.index()];
                    if refs[end] == self.own(c, side) {
                        self.dead[c][side.index()] = true;
                        self.queues[c][side.index()].clear();
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
    }

    fn consume(&mut self, t: usize, raws: &[u64]) {
        for raw in raws {
            self.live[t].remove(raw);
        }
        self.settle();
    }

    fn pick_raw(&mut self, t: usize) -> u64 {
        let roll = self.rng.below(100);
        if roll < 78 && !self.live[t].is_empty() {
            let at = self.rng.below(self.live[t].len());
            *self.live[t].keys().nth(at).unwrap()
        } else if roll < 88 && !self.issued[t].is_empty() {
            let at = self.rng.below(self.issued[t].len());
            self.issued[t][at]
        } else if roll < 96 {
            ((self.rng.next() % 3) << 16) | (1 + self.rng.below(60) as u64)
        } else {
            0
        }
    }

    /// Uma tabela que tem handle de canal (com mensagem esperando, se `queued`), quase sempre.
    fn table_with(&mut self, queued: bool) -> usize {
        let start = self.rng.below(3);
        for offset in 0..3 {
            let t = (start + offset) % 3;
            let found = self.live[t].values().any(|(ent, _)| {
                self.ents[*ent]
                    .end
                    .is_some_and(|(c, side)| !queued || !self.queues[c][side.index()].is_empty())
            });
            if found && !self.rng.chance(5) {
                return t;
            }
        }
        start
    }

    /// Um handle de canal vivo, quase sempre: é o que a escrita e a leitura precisam.
    fn pick_channel(&mut self, t: usize) -> u64 {
        let channels: Vec<u64> = self.live[t]
            .iter()
            .filter(|(_, (ent, _))| self.ents[*ent].end.is_some())
            .map(|(raw, _)| *raw)
            .collect();
        if channels.is_empty() || self.rng.chance(8) {
            return self.pick_raw(t);
        }
        channels[self.rng.below(channels.len())]
    }

    /// Um handle vivo que pode viajar, quase sempre.
    fn pick_transferable(&mut self, t: usize) -> u64 {
        let ready: Vec<u64> = self.live[t]
            .iter()
            .filter(|(_, (_, rights))| rights.contains(Rights::TRANSFER))
            .map(|(raw, _)| *raw)
            .collect();
        if ready.is_empty() || self.rng.chance(15) {
            return self.pick_raw(t);
        }
        ready[self.rng.below(ready.len())]
    }

    /// Um handle de canal com mensagem esperando na fila dele, quase sempre.
    fn pick_readable(&mut self, t: usize) -> u64 {
        let ready: Vec<u64> = self.live[t]
            .iter()
            .filter(|(_, (ent, _))| {
                self.ents[*ent]
                    .end
                    .is_some_and(|(c, side)| !self.queues[c][side.index()].is_empty())
            })
            .map(|(raw, _)| *raw)
            .collect();
        if ready.is_empty() || self.rng.chance(10) {
            return self.pick_channel(t);
        }
        ready[self.rng.below(ready.len())]
    }

    fn rights_mask(&mut self, base: Rights) -> Rights {
        if self.rng.chance(30) {
            return base;
        }
        let bits = base.bits() & self.rng.next() as u32;
        Rights::from_bits(bits).unwrap()
    }

    fn step(&mut self) {
        let roll = self.rng.below(100);
        let t = self.rng.below(3);
        match roll {
            0..=17 => self.op_insert(t),
            18..=27 => self.op_channel(t),
            28..=33 => self.op_close(t),
            34..=41 => self.op_duplicate(t),
            42..=46 => self.op_replace(t),
            47..=72 => {
                let t = self.table_with(false);
                self.op_write(t);
            }
            73..=94 => {
                let t = self.table_with(true);
                self.op_read(t);
            }
            _ => self.op_reserve_cancel(t),
        }
        if self.log.len().is_multiple_of(25) {
            self.check();
        }
    }

    fn op_insert(&mut self, t: usize) {
        self.log.push("insert");
        let kind = [
            ObjectKind::Process,
            ObjectKind::Thread,
            ObjectKind::Vmo,
            ObjectKind::Vmar,
            ObjectKind::Console,
        ][self.rng.below(5)];
        let root = kind.rights();
        let rights = self.rights_mask(root);
        let obj = TestObj::plain(kind, self.ents.len());
        let expected = self.live[t].len() < self.max_handles;
        let real = self.tabs[t].with(|table| table.insert(obj.clone(), rights));
        assert_eq!(real.is_ok(), expected);
        if let Ok(handle) = real {
            self.ents.push(Ent {
                obj,
                root,
                end: None,
            });
            let ent = self.ents.len() - 1;
            self.issue(t, handle);
            self.live[t].insert(handle.raw(), (ent, rights));
        }
    }

    fn op_channel(&mut self, t: usize) {
        self.log.push("channel");
        let room = self.live[t].len() + 2 <= self.max_handles;
        let first = self.tabs[t].with(Table::reserve);
        let (a, b) = new_channel();
        let Ok(first) = first else {
            assert!(!room);
            return;
        };
        let second = self.tabs[t].with(Table::reserve);
        let Ok(second) = second else {
            assert!(!room);
            self.tabs[t].with(|table| table.cancel(first));
            return;
        };
        assert!(room);
        let (v1, v2) = (first.handle(), second.handle());
        self.issue(t, v1);
        self.issue(t, v2);
        if self.rng.chance(20) {
            self.tabs[t].with(|table| {
                table.cancel(first);
                table.cancel(second);
            });
            return;
        }
        let (h1, h2) = self.tabs[t].with(|table| {
            (
                table.commit(first, a.clone(), Rights::CHANNEL),
                table.commit(second, b.clone(), Rights::CHANNEL),
            )
        });
        assert_eq!((h1, h2), (v1, v2));
        let chan = self.chans.len();
        for (obj, side) in [(a, Side::A), (b, Side::B)] {
            self.ents.push(Ent {
                obj,
                root: Rights::CHANNEL,
                end: Some((chan, side)),
            });
        }
        let n = self.ents.len();
        self.chans.push([n - 2, n - 1]);
        self.queues.push([VecDeque::new(), VecDeque::new()]);
        self.dead.push([false, false]);
        self.live[t].insert(v1.raw(), (n - 2, Rights::CHANNEL));
        self.live[t].insert(v2.raw(), (n - 1, Rights::CHANNEL));
    }

    fn close_real(&mut self, t: usize, raw: u64) -> Result<(), Status> {
        super::support::close_handle(&self.tabs[t], to_handle(raw))
    }

    fn op_close(&mut self, t: usize) {
        self.log.push("close");
        let raw = self.pick_raw(t);
        let expected = if self.live[t].contains_key(&raw) {
            Ok(())
        } else {
            Err(Status::BadHandle)
        };
        assert_eq!(self.close_real(t, raw), expected);
        self.consume(t, &[raw]);
    }

    fn wanted_rights(&mut self, t: usize, raw: u64) -> (u32, Result<Rights, Status>) {
        let have = self.live[t]
            .get(&raw)
            .map_or(Rights::ALL, |(_, rights)| *rights);
        let bits = match self.rng.below(10) {
            0 => self.rng.next() as u32,
            1..=6 => have.bits() & self.rng.next() as u32,
            _ => have.bits(),
        };
        (bits, parse(bits))
    }

    fn op_duplicate(&mut self, t: usize) {
        self.log.push("duplicate");
        let raw = self.pick_raw(t);
        let (_, parsed) = self.wanted_rights(t, raw);
        let real = parsed
            .and_then(|wanted| self.tabs[t].with(|table| table.duplicate(to_handle(raw), wanted)));
        let expected = match (parsed, self.live[t].get(&raw).copied()) {
            (Err(status), _) => Err(status),
            (Ok(_), None) => Err(Status::BadHandle),
            (Ok(_), Some((_, have))) if !have.contains(Rights::DUPLICATE) => {
                Err(Status::AccessDenied)
            }
            (Ok(wanted), Some((_, have))) if !wanted.is_subset_of(have) => Err(Status::InvalidArgs),
            (Ok(_), Some(_)) if self.live[t].len() >= self.max_handles => Err(Status::NoMemory),
            (Ok(wanted), Some((ent, _))) => {
                let new = real.expect("duplicate devia dar certo");
                self.issue(t, new);
                self.live[t].insert(new.raw(), (ent, wanted));
                return;
            }
        };
        assert_eq!(real, expected);
    }

    fn op_replace(&mut self, t: usize) {
        self.log.push("replace");
        let raw = self.pick_raw(t);
        let (_, parsed) = self.wanted_rights(t, raw);
        let Ok(wanted) = parsed else {
            return;
        };
        let real = self.tabs[t].with(|table| table.replace(to_handle(raw), wanted));
        let old = self.live[t].get(&raw).copied();
        let (real, closed) = match real {
            Ok(handle) => (Ok(handle), None),
            Err(failed) => (Err(failed.status), failed.closed),
        };
        match old {
            None => {
                assert_eq!(real, Err(Status::BadHandle));
                assert!(closed.is_none());
            }
            Some((ent, have)) => {
                self.live[t].remove(&raw);
                if wanted.is_subset_of(have) {
                    let new = real.expect("replace devia dar certo");
                    assert!(closed.is_none());
                    self.issue(t, new);
                    self.live[t].insert(new.raw(), (ent, wanted));
                } else {
                    assert_eq!(real, Err(Status::InvalidArgs));
                }
            }
        }
        if let Some(transit) = closed {
            close(transit, &mut Stack::default());
        }
        self.settle();
    }

    fn model_write(&mut self, t: usize, ch: u64, bytes: &[u8], arr: &[u64]) -> Result<(), Status> {
        if arr.len() > 8 || bytes.len() > CHANNEL_MAX_BYTES {
            self.consume(t, arr);
            return Err(Status::InvalidArgs);
        }
        let Some(&(ent, rights)) = self.live[t].get(&ch) else {
            self.consume(t, arr);
            return Err(Status::BadHandle);
        };
        let Some((c, side)) = self.ents[ent].end else {
            self.consume(t, arr);
            return Err(Status::WrongType);
        };
        if !rights.contains(Rights::WRITE) {
            self.consume(t, arr);
            return Err(Status::AccessDenied);
        }
        let dest = side.other();
        if self.dead[c][dest.index()] {
            self.consume(t, arr);
            return Err(Status::PeerClosed);
        }
        if self.queues[c][dest.index()].len() >= CHANNEL_QUEUE_LEN {
            return Err(Status::ShouldWait);
        }
        for (position, raw) in arr.iter().enumerate() {
            let error = match self.live[t].get(raw) {
                None => Some(Status::BadHandle),
                Some(_) if arr[..position].contains(raw) => Some(Status::BadHandle),
                Some((_, have)) if !have.contains(Rights::TRANSFER) => Some(Status::AccessDenied),
                Some((e, _)) if self.ents[*e].end == Some((c, side)) => Some(Status::NotSupported),
                Some(_) => None,
            };
            if let Some(error) = error {
                self.consume(t, arr);
                return Err(error);
            }
        }
        let handles = arr
            .iter()
            .map(|raw| self.live[t].remove(raw).unwrap())
            .collect();
        self.queues[c][dest.index()].push_back(Msg {
            bytes: bytes.to_vec(),
            handles,
        });
        self.settle();
        Ok(())
    }

    fn op_write(&mut self, t: usize) {
        self.log.push("write");
        let ch = self.pick_channel(t);
        let mut arr: Vec<u64> = if self.rng.chance(10) {
            (0..9).map(|_| self.pick_transferable(t)).collect()
        } else {
            let n = self.rng.below(5);
            (0..n).map(|_| self.pick_transferable(t)).collect()
        };
        if !arr.is_empty() && self.rng.chance(8) {
            arr.push(arr[0]);
        }
        let len = if self.rng.chance(3) {
            CHANNEL_MAX_BYTES + 1
        } else {
            self.rng.below(40)
        };
        let bytes: Vec<u8> = (0..len).map(|_| self.rng.next() as u8).collect();
        let expected = self.model_write(t, ch, &bytes, &arr);
        let handles: Vec<Handle> = arr.iter().map(|&raw| to_handle(raw)).collect();
        let real = write(&self.tabs[t], to_handle(ch), &bytes, &handles);
        assert_eq!(real, expected, "write ({:?})", self.log.len());
        if real.is_ok() {
            self.stats.writes_ok += 1;
            self.stats.writes_with_handles += usize::from(!arr.is_empty());
        } else {
            self.stats.write_errors += 1;
        }
    }

    fn op_read(&mut self, t: usize) {
        self.log.push("read");
        let ch = self.pick_readable(t);
        let bytes_cap = [0, 3, 40, 1024, 1024, 1024][self.rng.below(6)];
        let handles_cap = [0, 2, 8, 8, 8][self.rng.below(5)];
        let fail_copy = self.rng.chance(15);
        let mut plan = Ok(());
        let mut sizes = (0, 0);
        match self.live[t].get(&ch).copied() {
            None => plan = Err(Status::BadHandle),
            Some((ent, rights)) => match self.ents[ent].end {
                None => plan = Err(Status::WrongType),
                Some(_) if !rights.contains(Rights::READ) => plan = Err(Status::AccessDenied),
                Some((c, side)) => match self.queues[c][side.index()].front() {
                    None if self.dead[c][side.other().index()] => plan = Err(Status::PeerClosed),
                    None => plan = Err(Status::ShouldWait),
                    Some(msg) => {
                        sizes = (msg.bytes.len(), msg.handles.len());
                        if sizes.0 > bytes_cap || sizes.1 > handles_cap {
                            plan = Err(Status::BufferTooSmall);
                        } else if self.live[t].len() + sizes.1 > self.max_handles {
                            plan = Err(Status::NoMemory);
                        } else if fail_copy {
                            plan = Err(Status::OutOfRange);
                        }
                    }
                },
            },
        }
        let mut got = None;
        let real = channel_read(
            &self.tabs[t],
            to_handle(ch),
            bytes_cap,
            handles_cap,
            |bytes, handles| {
                got = Some((bytes.to_vec(), handles.to_vec()));
                !fail_copy
            },
        );
        match (plan, real) {
            (Ok(()), Ok(actual)) => {
                let (c, side) = self.ents[self.live[t][&ch].0].end.unwrap();
                let msg = self.queues[c][side.index()].pop_front().unwrap();
                let (bytes, handles) = got.unwrap();
                assert_eq!(bytes, msg.bytes);
                assert_eq!((actual.bytes, actual.handles), sizes);
                assert_eq!(handles.len(), msg.handles.len());
                for (handle, entry) in handles.into_iter().zip(msg.handles) {
                    self.issue(t, handle);
                    self.live[t].insert(handle.raw(), entry);
                }
                self.settle();
                self.stats.reads_ok += 1;
                self.stats.reads_with_handles += usize::from(sizes.1 > 0);
            }
            (Err(status), Err(failed)) => {
                self.stats.read_errors += 1;
                assert_eq!(failed.status, status, "read");
                if status == Status::BufferTooSmall {
                    assert_eq!((failed.actual.bytes, failed.actual.handles), sizes);
                }
            }
            (plan, real) => panic!("read: esperado {plan:?}, veio {real:?}"),
        }
    }

    fn op_reserve_cancel(&mut self, t: usize) {
        self.log.push("reserve");
        let expected = self.live[t].len() < self.max_handles;
        let real = self.tabs[t].with(Table::reserve);
        assert_eq!(real.is_ok(), expected);
        if let Ok(reservation) = real {
            let value = reservation.handle();
            self.issue(t, value);
            self.tabs[t].with(|table| {
                assert!(table.get(value, Rights::NONE).is_err());
                table.cancel(reservation);
            });
        }
    }

    fn check(&self) {
        let context = format!(
            "depois de {} ops, última {:?}",
            self.log.len(),
            self.log.last()
        );
        for (t, tab) in self.tabs.iter().enumerate() {
            tab.with(|table| {
                assert_eq!(table.len(), self.live[t].len(), "{context}");
                assert_eq!(table.reserved(), 0, "{context}");
                for (&raw, &(ent, rights)) in &self.live[t] {
                    let granted = table.get(to_handle(raw), Rights::NONE).expect(&context);
                    assert!(same(granted.object, &self.ents[ent].obj), "{context}");
                    assert_eq!(granted.rights, rights, "{context}");
                    assert!(
                        rights.is_subset_of(self.ents[ent].root),
                        "direito cresceu: {context}"
                    );
                }
            });
        }
        let refs = self.all_refs();
        for ent in 0..self.ents.len() {
            assert_eq!(
                count(&self.ents[ent].obj),
                refs[ent],
                "handle_count do objeto {ent}: {context}"
            );
        }
        for c in 0..self.chans.len() {
            let obj = &self.ents[self.chans[c][0]].obj;
            for side in [Side::A, Side::B] {
                let (dead, queued, own, front) = obj
                    .with_channel(|chan| {
                        (
                            chan.is_dead(side),
                            chan.queued(side),
                            chan.own_in_queue(side),
                            chan.peek(side)
                                .ok()
                                .map(|peek| (peek.bytes.to_vec(), peek.handles)),
                        )
                    })
                    .unwrap();
                let queue = &self.queues[c][side.index()];
                assert_eq!(dead, self.dead[c][side.index()], "ponta morta: {context}");
                assert_eq!(queued, queue.len(), "fila: {context}");
                assert_eq!(own, self.own(c, side), "próprios na fila: {context}");
                let model_front = queue
                    .front()
                    .map(|msg| (msg.bytes.clone(), msg.handles.len()));
                assert_eq!(front, model_front, "frente da fila: {context}");
            }
        }
    }

    fn finish(mut self) {
        for t in 0..3 {
            let mut work = Stack::default();
            self.tabs[t].with(|table| table.drain(&mut work));
            close_pending(&mut work);
            self.live[t].clear();
        }
        self.settle();
        self.log.push("fim");
        self.check();
        // Ciclos entre filas de canais não são coletados (SDD 17): o que sobra está em fila viva.
        let refs = self.all_refs();
        for (ent, left) in refs.iter().enumerate() {
            assert_eq!(count(&self.ents[ent].obj), *left);
        }
        assert!(self.live.iter().all(BTreeMap::is_empty));
    }
}

fn run(seed: u64, ops: usize, max_handles: usize, max_generation: u64) {
    let mut world = World::new(seed, max_handles, max_generation);
    for _ in 0..ops {
        world.step();
    }
    let (retired, deaths) = (
        world
            .tabs
            .iter()
            .map(|tab| tab.with(|t| t.retired()))
            .sum::<usize>(),
        world.dead.iter().flatten().filter(|dead| **dead).count(),
    );
    let stats = std::mem::take(&mut world.stats);
    world.finish();
    assert!(stats.writes_ok > 150 && stats.writes_with_handles > 50);
    assert!(stats.reads_ok > 100 && stats.reads_with_handles > 30);
    assert!(stats.write_errors > 300 && stats.read_errors > 300);
    assert!(deaths > 50, "poucas pontas morreram: {deaths}");
    if max_generation < 100 {
        assert!(retired > 10, "nenhum slot se aposentou");
    }
}

#[test]
fn random_operations_agree_with_the_naive_model_seed_1() {
    run(1, 20_000, 40, Limits::ABI.max_generation);
}

#[test]
fn random_operations_agree_with_the_naive_model_seed_2() {
    run(0xDEAD_BEEF, 20_000, 24, Limits::ABI.max_generation);
}

#[test]
fn random_operations_agree_with_the_naive_model_with_a_tiny_generation_budget() {
    run(7, 20_000, 40, 6);
}

#[test]
fn random_operations_agree_with_the_naive_model_with_a_tight_table() {
    run(99, 20_000, 8, Limits::ABI.max_generation);
}
