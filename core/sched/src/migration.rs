//! Migração entre duas filas: a tarefa nunca some nem está nas duas ao mesmo tempo, e o
//! lag viaja com ela. Cada fila é um `Composite`, como em cada CPU do kernel.

use crate::TaskId;
use crate::composite::Composite;
use crate::eevdf::Eevdf;
use crate::scheduler::{SchedParams, Scheduler};

type Queue = Composite<8, 16>;

const IDS: u32 = 12;

fn id(raw: u32) -> TaskId {
    TaskId::new(raw)
}

fn lag_of<const N: usize>(sched: &mut Eevdf<N>, task: TaskId) -> i64 {
    sched.lag_ns(task).expect("tarefa na fila")
}

#[test]
fn lag_travels_with_a_ready_task_to_another_queue() {
    let mut from = Eevdf::<16>::new();
    let mut to = Eevdf::<16>::new();
    for raw in 0..3 {
        from.add(id(raw), SchedParams::fair(0), 0).unwrap();
    }
    to.add(id(9), SchedParams::fair(0), 0).unwrap();
    let mut now = 0;
    for _ in 0..5 {
        from.pick_next(now);
        now += 700_000;
        from.tick(now);
    }
    from.pick_next(now);
    let moved = from
        .pull_candidate(|task| from.current() != Some(task))
        .expect("alguém pronto");
    let before = lag_of(&mut from, moved);
    let detached = from.detach(moved).expect("pronta sai");
    assert_eq!(detached.lag_ns, before);
    assert!(!from.contains(moved));
    to.add_with_lag(moved, detached.params, detached.lag_ns, now)
        .unwrap();
    let after = lag_of(&mut to, moved);
    let tolerance = before.abs() / 4 + 1_000;
    assert!(
        (after - before).abs() <= tolerance,
        "lag {before} virou {after}"
    );
}

#[test]
fn the_current_task_cannot_be_detached() {
    let mut queue = Queue::new();
    queue.add(id(1), SchedParams::fair(0), 0).unwrap();
    queue.add(id(2), SchedParams::fifo(5), 0).unwrap();
    let current = queue.pick_next(1).unwrap();
    assert_eq!(current, id(2));
    assert!(queue.detach(current).is_none());
    assert!(queue.contains_for_test(current));
    assert!(queue.detach(id(1)).is_some());
}

#[test]
fn a_sleeping_task_detaches_with_its_saved_lag() {
    let mut from = Eevdf::<16>::new();
    from.add(id(1), SchedParams::fair(0), 0).unwrap();
    from.add(id(2), SchedParams::fair(0), 0).unwrap();
    from.pick_next(0);
    from.tick(700_000);
    from.sleep(id(1), 700_000);
    let detached = from.detach(id(1)).expect("dormindo sai");
    assert_eq!(
        detached.params,
        SchedParams::fair_with_slice(0, crate::eevdf::BASE_SLICE_NS)
    );
    assert!(!from.contains(id(1)));
}

#[test]
fn pull_candidate_skips_the_current_and_what_the_filter_refuses() {
    let mut queue = Eevdf::<16>::new();
    for raw in 1..=4 {
        queue.add(id(raw), SchedParams::fair(0), 0).unwrap();
    }
    let current = queue.pick_next(0).unwrap();
    let mut seen = Vec::new();
    let picked = queue.pull_candidate(|task| {
        seen.push(task);
        task != id(3)
    });
    assert!(!seen.contains(&current));
    assert_ne!(picked, Some(current));
    assert_ne!(picked, Some(id(3)));
    assert!(picked.is_some());
    assert_eq!(queue.pull_candidate(|_| false), None);
}

#[test]
fn real_time_tasks_migrate_by_detach_but_are_not_pull_candidates() {
    let mut queue = Queue::new();
    queue.add(id(1), SchedParams::fifo(10), 0).unwrap();
    queue.add(id(2), SchedParams::fifo(20), 0).unwrap();
    queue.pick_next(0);
    assert_eq!(queue.pull_candidate(|_| true), None);
    let moved = queue.detach(id(1)).expect("pronta sai");
    assert_eq!(moved.params, SchedParams::fifo(10));
    let mut other = Queue::new();
    other.add_with_lag(id(1), moved, 0).unwrap();
    assert_eq!(other.runnable(), 1);
}

struct Rng(u64);

impl Rng {
    fn below(&mut self, bound: u64) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) % bound
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Where {
    Nowhere,
    In(usize),
}

#[test]
fn migration_never_loses_a_task_nor_puts_it_in_two_queues() {
    for seed in 1..=200_u64 {
        let mut queues = [Queue::new(), Queue::new()];
        let mut place = [Where::Nowhere; IDS as usize];
        let mut rng = Rng(seed * 0x9E37_79B9);
        let mut now = 0_u64;
        for step in 0..500 {
            now += rng.below(1_000_000);
            let raw = u32::try_from(rng.below(u64::from(IDS))).unwrap();
            let task = id(raw);
            let at = raw as usize;
            let queue = usize::from(rng.below(2) == 0);
            let context = format!("semente {seed}, passo {step}, tarefa {raw}");
            match rng.below(100) {
                0..=14 if place[at] == Where::Nowhere => {
                    let params = if rng.below(5) == 0 {
                        SchedParams::fifo(1 + u8::try_from(rng.below(99)).unwrap())
                    } else {
                        SchedParams::fair(i8::try_from(rng.below(40)).unwrap() - 20)
                    };
                    queues[queue].add(task, params, now).unwrap();
                    place[at] = Where::In(queue);
                }
                15..=24 => {
                    if let Where::In(owner) = place[at] {
                        queues[owner].remove(task);
                        place[at] = Where::Nowhere;
                    }
                }
                25..=34 => {
                    if let Where::In(owner) = place[at] {
                        queues[owner].sleep(task, now);
                    }
                }
                35..=44 => {
                    if let Where::In(owner) = place[at] {
                        queues[owner].wake(task, now);
                    }
                }
                45..=64 => {
                    if let Where::In(owner) = place[at] {
                        let target = 1 - owner;
                        match queues[owner].detach(task) {
                            Some(moved) => {
                                queues[target].add_with_lag(task, moved, now).unwrap();
                                place[at] = Where::In(target);
                            }
                            None => assert_eq!(queues[owner].current(), Some(task), "{context}"),
                        }
                    }
                }
                65..=74 => {
                    let (thief, victim) = (queue, 1 - queue);
                    let current = queues[victim].current();
                    let picked = queues[victim].pull_candidate(|task| Some(task) != current);
                    if let Some(task) = picked {
                        let moved = queues[victim].detach(task).expect("pronta sai");
                        queues[thief].add_with_lag(task, moved, now).unwrap();
                        place[task.as_u32() as usize] = Where::In(thief);
                    }
                }
                75..=84 => {
                    queues[queue].tick(now);
                }
                _ => {
                    queues[queue].pick_next(now);
                }
            }
            for (index, &here) in place.iter().enumerate() {
                let task = id(u32::try_from(index).unwrap());
                for (q, queue) in queues.iter().enumerate() {
                    let present = queue.contains_for_test(task);
                    assert_eq!(present, here == Where::In(q), "{context}: tarefa {index}");
                }
            }
            for queue in &queues {
                queue.fair().check_invariants();
            }
        }
    }
}
