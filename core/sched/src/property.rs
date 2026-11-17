//! Teste de propriedade: eventos aleatórios contra um modelo simples e, no EEVDF,
//! contra a recontagem completa dos invariantes. A semente é fixa e vai na falha.

use crate::TaskId;
use crate::composite::Composite;
use crate::eevdf::Eevdf;
use crate::fifo::Fifo;
use crate::round_robin::RoundRobin;
use crate::scheduler::{SchedError, SchedParams, Scheduler};

const IDS: u32 = 24;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Model {
    Absent,
    Runnable,
    Sleeping,
}

#[derive(Clone, Copy)]
enum Classes {
    Fair,
    Fifo,
    Both,
}

fn random_params(rng: &mut Rng, classes: Classes) -> SchedParams {
    let fifo = match classes {
        Classes::Fair => false,
        Classes::Fifo => true,
        Classes::Both => rng.below(4) == 0,
    };
    if fifo {
        SchedParams::fifo(u8::try_from(1 + rng.below(99)).unwrap())
    } else {
        let nice = i8::try_from(rng.below(40)).unwrap() - 20;
        match rng.below(3) {
            0 => SchedParams::fair_with_slice(nice, 100_000 + rng.below(5_000_000)),
            _ => SchedParams::fair(nice),
        }
    }
}

fn fuzz<S: Scheduler>(sched: &mut S, seed: u64, steps: u32, classes: Classes, check: &dyn Fn(&S)) {
    let mut rng = Rng(seed);
    let mut model = [Model::Absent; IDS as usize];
    let mut now = 0_u64;
    for step in 0..steps {
        now += rng.below(1_500_000);
        let raw = u32::try_from(rng.below(u64::from(IDS))).unwrap();
        let (id, at) = (TaskId::new(raw), raw as usize);
        let what = rng.below(100);
        let context = format!("semente {seed:#x}, passo {step}, evento {what}, tarefa {raw}");
        match what {
            0..=14 => match sched.add(id, random_params(&mut rng, classes), now) {
                Ok(()) => {
                    assert_eq!(model[at], Model::Absent, "{context}");
                    model[at] = Model::Runnable;
                }
                Err(SchedError::Duplicate) => assert_ne!(model[at], Model::Absent, "{context}"),
                Err(SchedError::Full) => assert_eq!(model[at], Model::Absent, "{context}"),
                Err(error) => panic!("{error:?}: {context}"),
            },
            15..=22 => {
                sched.remove(id);
                model[at] = Model::Absent;
            }
            23..=42 => {
                sched.wake(id, now);
                if model[at] == Model::Sleeping {
                    model[at] = Model::Runnable;
                }
            }
            43..=57 => {
                sched.sleep(id, now);
                if model[at] == Model::Runnable {
                    model[at] = Model::Sleeping;
                }
            }
            58..=75 => {
                sched.tick(now);
            }
            76..=82 => sched.yield_current(now),
            _ => {
                let picked = sched.pick_next(now);
                let any = model.contains(&Model::Runnable);
                assert_eq!(picked.is_some(), any, "{context}");
                if let Some(task) = picked {
                    assert_eq!(model[task.as_u32() as usize], Model::Runnable, "{context}");
                }
                assert_eq!(sched.current(), picked, "{context}");
                assert!(!sched.needs_resched() || picked.is_none(), "{context}");
            }
        }
        if let Some(task) = sched.current() {
            let state = model[task.as_u32() as usize];
            assert_eq!(
                state,
                Model::Runnable,
                "corrente dormindo ou ausente: {context}"
            );
        }
        if sched.current().is_none() && model.contains(&Model::Runnable) {
            assert!(sched.needs_resched(), "CPU parada com trabalho: {context}");
        }
        check(sched);
    }
}

#[test]
fn eevdf_never_picks_a_sleeper_and_keeps_every_invariant() {
    for seed in 1..=300_u64 {
        let mut sched = Eevdf::<16>::new();
        fuzz(&mut sched, seed * 0x9E37_79B9, 400, Classes::Fair, &|s| {
            s.check_invariants();
        });
    }
}

#[test]
fn eevdf_with_one_slot_and_with_many_survives_the_same_events() {
    for seed in 1..=100_u64 {
        let mut one = Eevdf::<1>::new();
        fuzz(&mut one, seed, 200, Classes::Fair, &|s| {
            s.check_invariants();
        });
        let mut many = Eevdf::<64>::new();
        fuzz(&mut many, seed, 600, Classes::Fair, &|s| {
            s.check_invariants();
        });
    }
}

#[test]
fn round_robin_never_picks_a_sleeper() {
    for seed in 1..=200_u64 {
        fuzz(
            &mut RoundRobin::<16>::new(),
            seed,
            400,
            Classes::Both,
            &|_| {},
        );
    }
}

#[test]
fn fifo_never_picks_a_sleeper_nor_a_lower_priority_than_the_waiting() {
    for seed in 1..=200_u64 {
        fuzz(&mut Fifo::<16>::new(), seed, 400, Classes::Fifo, &|_| {});
    }
}

#[test]
fn composite_never_picks_a_sleeper_and_keeps_the_fair_invariants() {
    for seed in 1..=200_u64 {
        let mut sched = Composite::<8, 16>::new();
        fuzz(&mut sched, seed, 500, Classes::Both, &|s| {
            s.fair().check_invariants();
        });
    }
}
