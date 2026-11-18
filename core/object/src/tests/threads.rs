//! Threads de verdade, como pede o guidelines 07: duas tabelas e um canal entre elas, com a
//! ordem de lock tabela, depois canal, e o fechamento sempre fora dos dois.

use std::sync::mpsc;
use std::sync::{Arc, Barrier};
use std::thread;

use rustos_hal::abi::{Handle, Rights, Status};

use super::support::{Tab, TestObj, close_handle, count, handle_of, new_channel, read, tab, write};
use crate::{Lock, Object, ObjectKind, Side};

const ROUNDS: usize = 3000;

fn spawn_writer(p: &Arc<Tab>, ha: Handle, objects: Vec<TestObj>) -> thread::JoinHandle<usize> {
    let p = Arc::clone(p);
    thread::spawn(move || {
        let mut sent = 0;
        for object in &objects {
            let handle = handle_of(&p, object, Rights::TRANSFER);
            loop {
                match write(&p, ha, b"payload", &[handle]) {
                    Ok(()) => {
                        sent += 1;
                        break;
                    }
                    Err(Status::ShouldWait) => thread::yield_now(),
                    Err(Status::PeerClosed) => return sent,
                    Err(other) => panic!("escrita falhou: {other:?}"),
                }
            }
        }
        sent
    })
}

#[test]
fn a_writer_a_reader_and_a_handle_churn_on_the_same_table_lose_nothing() {
    let (a, b) = new_channel();
    let (p, q) = (Arc::new(tab()), Arc::new(tab()));
    let ha = handle_of(&p, &a, Rights::CHANNEL);
    let hb = handle_of(&q, &b, Rights::CHANNEL);
    let objects: Vec<_> = (0..ROUNDS)
        .map(|id| TestObj::plain(ObjectKind::Vmo, id))
        .collect();
    let writer = spawn_writer(&p, ha, objects.clone());
    let churn = {
        let p = Arc::clone(&p);
        thread::spawn(move || {
            for _ in 0..ROUNDS {
                let copy = p
                    .with(|t| t.duplicate(ha, Rights::DUPLICATE.union(Rights::WRITE)))
                    .unwrap();
                close_handle(&p, copy).unwrap();
            }
        })
    };
    let reader = {
        let q = Arc::clone(&q);
        thread::spawn(move || {
            let mut received = 0;
            while received < ROUNDS {
                match read(&q, hb, 64, 8) {
                    Ok((bytes, handles)) => {
                        assert_eq!(bytes, b"payload");
                        for handle in handles {
                            close_handle(&q, handle).unwrap();
                            received += 1;
                        }
                    }
                    Err(failed) if failed.status == Status::ShouldWait => thread::yield_now(),
                    Err(failed) => panic!("leitura falhou: {:?}", failed.status),
                }
            }
        })
    };
    assert_eq!(writer.join().unwrap(), ROUNDS);
    churn.join().unwrap();
    reader.join().unwrap();
    assert!(objects.iter().all(|object| count(object) == 0));
    assert_eq!((p.with(|t| t.len()), q.with(|t| t.len())), (1, 1));
    assert_eq!((count(&a), count(&b)), (1, 1));
}

#[test]
fn closing_the_peer_while_handles_travel_closes_every_handle_exactly_once() {
    for round in 0..20 {
        let (a, b) = new_channel();
        let (p, q) = (Arc::new(tab()), Arc::new(tab()));
        let ha = handle_of(&p, &a, Rights::CHANNEL);
        let hb = handle_of(&q, &b, Rights::CHANNEL);
        let objects: Vec<_> = (0..500)
            .map(|id| TestObj::plain(ObjectKind::Vmo, id))
            .collect();
        let writer = spawn_writer(&p, ha, objects.clone());
        let closer = {
            let q = Arc::clone(&q);
            thread::spawn(move || {
                for _ in 0..(round * 7) {
                    thread::yield_now();
                }
                let _ = read(&q, hb, 64, 8);
                close_handle(&q, hb).unwrap();
            })
        };
        let sent = writer.join().unwrap();
        closer.join().unwrap();
        assert!(sent <= 500);
        // Os que o leitor tirou e os que ficaram na fila ou foram recusados: nenhum vaza.
        let live_in_q = q.with(|t| t.len());
        let leaked = objects.iter().filter(|object| count(object) != 0).count();
        assert_eq!(leaked, live_in_q, "rodada {round}");
        assert_eq!(count(&b), 0);
    }
}

#[test]
fn the_end_died_hook_ignores_the_peer_lock_held_by_another_thread() {
    let (a, _b) = new_channel();
    let end = Arc::clone(a.end_ref().unwrap());
    let held = Arc::new(Barrier::new(2));
    let (release, wait) = mpsc::channel::<()>();
    let holder = {
        let (a, held) = (a.clone(), Arc::clone(&held));
        thread::spawn(move || {
            a.with_channel(|_| {
                held.wait();
                wait.recv().unwrap();
            })
        })
    };
    held.wait();
    a.on_end_died(Side::B);
    release.send(()).unwrap();
    holder.join().unwrap();
    assert_eq!(
        end.peer_closed_calls
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
}

#[test]
#[should_panic(expected = "gancho com o lock do par preso nesta thread")]
fn the_end_died_hook_inside_the_peer_lock_of_the_same_thread_fails() {
    let (a, _b) = new_channel();
    a.with_channel(|_| a.on_end_died(Side::B));
}
