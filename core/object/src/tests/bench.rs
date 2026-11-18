//! Medida de host, só com `--ignored`: `cargo test --release -p rustos-object bench --
//! --ignored --nocapture --test-threads=1`. Não é QEMU nem metal.

use core::hint::black_box;
use std::sync::Arc;
use std::time::Instant;

use rustos_hal::abi::{Handle, Rights};

use super::support::{Locked, Stack, Table, TestObj, VecSlots};
use crate::{HandleTable, Lock, ObjectKind, channel_read, channel_write, close};

const ROUNDS: usize = 9;
const ITERS: u64 = 200_000;

/// O melhor tempo por chamada, em décimos de nanossegundo.
fn best_ns(mut body: impl FnMut()) -> u128 {
    let mut best = u128::MAX;
    for _ in 0..ROUNDS {
        let start = Instant::now();
        for _ in 0..ITERS {
            body();
        }
        best = best.min(start.elapsed().as_nanos() * 10 / u128::from(ITERS));
    }
    best
}

#[test]
#[ignore = "medida de host"]
fn bench_handle_table_and_channel() {
    let mut table: Table = HandleTable::new(VecSlots::default());
    let object = TestObj::plain(ObjectKind::Vmo, 1);
    let probe = table.insert(object.clone(), Rights::VMO).unwrap();
    for id in 0..1000 {
        table
            .insert(TestObj::plain(ObjectKind::Vmo, id), Rights::VMO)
            .unwrap();
    }
    let lookup = best_ns(|| {
        black_box(table.get(black_box(probe), Rights::READ).is_ok());
    });
    let insert_remove = best_ns(|| {
        let handle = table
            .insert(black_box(object.clone()), Rights::VMO)
            .unwrap();
        let transit = table.remove(handle).unwrap();
        close(transit, &mut Stack::default());
    });
    let clone_only = best_ns(|| {
        black_box(object.clone());
    });
    show("buscar com direito (1001 handles)", lookup);
    show("inserir + remover + fechar", insert_remove);
    show("clone do Arc sozinho", clone_only);

    let (a, b) = super::support::new_channel();
    let p = Locked::new(HandleTable::<TestObj, VecSlots>::new(VecSlots::default()));
    let q = Locked::new(HandleTable::<TestObj, VecSlots>::new(VecSlots::default()));
    let ha = p.with(|t| t.insert(a, Rights::CHANNEL)).unwrap();
    let hb = q.with(|t| t.insert(b, Rights::CHANNEL)).unwrap();
    let objects: Vec<_> = (0..4)
        .map(|id| TestObj::plain(ObjectKind::Vmo, id))
        .collect();
    let bytes: Arc<[u8]> = Arc::from(&[7_u8; 64][..]);
    let mut work = Stack::default();
    let write_read = best_ns(|| {
        let handles: Vec<_> = objects
            .iter()
            .map(|object| {
                p.with(|t| t.insert(object.clone(), Rights::TRANSFER))
                    .unwrap()
            })
            .collect();
        channel_write(&p, ha, Arc::clone(&bytes), &handles, &mut work).unwrap();
        let mut got = [Handle::INVALID; 4];
        let sizes = channel_read(&q, hb, 64, 8, |_, handles| {
            got[..handles.len()].copy_from_slice(handles);
            true
        })
        .unwrap();
        for handle in &got[..sizes.handles] {
            let transit = q.with(|t| t.remove(*handle)).unwrap();
            close(transit, &mut work);
        }
    });
    let setup_only = best_ns(|| {
        let handles: Vec<_> = objects
            .iter()
            .map(|object| {
                p.with(|t| t.insert(object.clone(), Rights::TRANSFER))
                    .unwrap()
            })
            .collect();
        for handle in handles {
            let transit = p.with(|t| t.remove(handle)).unwrap();
            close(transit, &mut work);
        }
    });
    show(
        "escrever + ler 1 mensagem de 64 B, 4 handles (com preparar e fechar)",
        write_read,
    );
    show("só preparar e fechar os 4 handles", setup_only);
    println!(
        "bytes: Slot {}, Message {}, Channel {}",
        core::mem::size_of::<crate::Slot<TestObj>>(),
        core::mem::size_of::<crate::Message<TestObj>>(),
        core::mem::size_of::<crate::Channel<TestObj>>(),
    );
}

fn show(what: &str, tenths: u128) {
    println!("{what}: {}.{} ns", tenths / 10, tenths % 10);
}
