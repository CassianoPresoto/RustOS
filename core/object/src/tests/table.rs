//! A tabela de handles: valores, geração, aposentadoria, teto, fila livre, reserva, e um teste
//! vermelho por regra de recusa (SDD 17, "Tabela de handles" e "Operações sobre handle").

use core::mem::size_of;

use rustos_hal::abi::{Handle, MAX_HANDLES, Rights, Status};

use super::support::{
    Stack, Table, TestObj, VecSlots, close_handle, count, lock_depth, tab, table,
};
use crate::{
    FixedSlots, HandleTable, Limits, Lock, Object, ObjectKind, Slot, close, close_pending,
};

fn plain(id: usize) -> TestObj {
    TestObj::plain(ObjectKind::Vmo, id)
}

fn insert(table: &mut Table, id: usize, rights: Rights) -> Handle {
    table.insert(plain(id), rights).unwrap()
}

fn close_one(table: &mut Table, handle: Handle) {
    let transit = table.remove(handle).unwrap();
    close(transit, &mut Stack::default());
}

#[test]
fn slot_is_32_bytes_with_the_16_byte_object_of_the_kernel() {
    assert_eq!(size_of::<TestObj>(), 16);
    assert_eq!(size_of::<Option<TestObj>>(), 16);
    assert_eq!(size_of::<Slot<TestObj>>(), 32);
    assert_eq!(32 * MAX_HANDLES, 128 * 1024);
}

#[test]
fn the_first_handle_is_one_and_zero_is_never_valid() {
    let mut table = table();
    let first = insert(&mut table, 1, Rights::VMO);
    assert_eq!(first.raw(), 1);
    assert_eq!(crate::handle_from_raw(0), Err(Status::BadHandle));
    assert_eq!(crate::handle_from_raw(1 << 16), Err(Status::BadHandle));
    assert_eq!(crate::handle_from_raw(1), Ok(first));
}

#[test]
fn lookup_refuses_unknown_stale_free_and_out_of_range_values() {
    let mut table = table();
    let live = insert(&mut table, 1, Rights::VMO);
    let gone = insert(&mut table, 2, Rights::VMO);
    close_one(&mut table, gone);
    let never = Handle::new(0, 9).unwrap();
    let next_generation_of_free_slot = Handle::new(1, gone.index()).unwrap();
    for handle in [
        gone,
        never,
        next_generation_of_free_slot,
        Handle::new(0, 65_000).unwrap(),
    ] {
        assert_eq!(
            table.get(handle, Rights::NONE).err(),
            Some(Status::BadHandle)
        );
        assert_eq!(table.remove(handle).err(), Some(Status::BadHandle));
        assert_eq!(
            table.duplicate(handle, Rights::NONE),
            Err(Status::BadHandle)
        );
    }
    assert!(table.get(live, Rights::READ).is_ok());
}

#[test]
fn a_wrong_generation_of_a_live_slot_is_bad_handle() {
    let mut table = table();
    let live = insert(&mut table, 1, Rights::VMO);
    let forged = Handle::new(live.generation() + 1, live.index()).unwrap();
    assert_eq!(
        table.get(forged, Rights::NONE).err(),
        Some(Status::BadHandle)
    );
}

#[test]
fn wrong_type_is_checked_before_rights() {
    let mut table = table();
    let handle = insert(&mut table, 1, Rights::READ);
    assert_eq!(
        table
            .get_of(handle, ObjectKind::Channel, Rights::WRITE)
            .err(),
        Some(Status::WrongType)
    );
    assert_eq!(
        table.get_of(handle, ObjectKind::Vmo, Rights::WRITE).err(),
        Some(Status::AccessDenied)
    );
    assert!(table.get_of(handle, ObjectKind::Vmo, Rights::READ).is_ok());
}

#[test]
fn a_missing_right_is_access_denied_and_none_is_always_met() {
    let mut table = table();
    let handle = insert(&mut table, 1, Rights::READ);
    assert_eq!(
        table.get(handle, Rights::WRITE).err(),
        Some(Status::AccessDenied)
    );
    assert!(table.get(handle, Rights::NONE).is_ok());
}

#[test]
fn duplicate_counts_the_new_handle_and_only_narrows() {
    let mut table = table();
    let object = plain(1);
    let full = table.insert(object.clone(), Rights::VMO).unwrap();
    assert_eq!(count(&object), 1);
    let wanted = Rights::READ.union(Rights::DUPLICATE);
    let narrow = table.duplicate(full, wanted).unwrap();
    assert_eq!(count(&object), 2);
    assert_eq!(table.get(narrow, Rights::NONE).unwrap().rights, wanted);
    let again = table.duplicate(narrow, Rights::READ).unwrap();
    assert_eq!(table.get(again, Rights::NONE).unwrap().rights, Rights::READ);
    assert_eq!(count(&object), 3);
}

#[test]
fn duplicate_without_the_right_is_access_denied() {
    let mut table = table();
    let object = plain(1);
    let handle = table.insert(object.clone(), Rights::READ).unwrap();
    assert_eq!(
        table.duplicate(handle, Rights::READ),
        Err(Status::AccessDenied)
    );
    assert_eq!(count(&object), 1);
}

#[test]
fn duplicate_with_more_rights_is_invalid_args() {
    let mut table = table();
    let object = plain(1);
    let handle = table
        .insert(object.clone(), Rights::DUPLICATE.union(Rights::READ))
        .unwrap();
    assert_eq!(
        table.duplicate(handle, Rights::WRITE),
        Err(Status::InvalidArgs)
    );
    assert_eq!(
        table.duplicate(
            handle,
            Rights::DUPLICATE.union(Rights::READ).union(Rights::WRITE)
        ),
        Err(Status::InvalidArgs)
    );
    assert_eq!(count(&object), 1);
    assert_eq!(table.len(), 1);
}

#[test]
fn replace_moves_the_object_to_a_new_value_without_touching_the_count() {
    let mut table = table();
    let object = plain(1);
    let old = table
        .insert(object.clone(), Rights::READ.union(Rights::WRITE))
        .unwrap();
    let new = table.replace(old, Rights::READ).unwrap();
    assert_ne!(new, old);
    assert_eq!(table.get(old, Rights::NONE).err(), Some(Status::BadHandle));
    assert_eq!(table.get(new, Rights::NONE).unwrap().rights, Rights::READ);
    assert_eq!(count(&object), 1);
    assert_eq!(table.len(), 1);
}

#[test]
fn replace_needs_no_right_not_even_duplicate() {
    let mut table = table();
    let old = table.insert(plain(1), Rights::READ).unwrap();
    assert!(table.replace(old, Rights::READ).is_ok());
}

#[test]
fn replace_with_more_rights_kills_the_old_handle_and_hands_it_back_to_close() {
    let mut table = table();
    let object = plain(1);
    let old = table.insert(object.clone(), Rights::READ).unwrap();
    let failed = table.replace(old, Rights::WRITE).unwrap_err();
    assert_eq!(failed.status, Status::InvalidArgs);
    assert_eq!(table.get(old, Rights::NONE).err(), Some(Status::BadHandle));
    assert!(table.is_empty());
    assert_eq!(count(&object), 1);
    close(failed.closed.unwrap(), &mut Stack::default());
    assert_eq!(count(&object), 0);
}

#[test]
fn replace_of_an_unknown_handle_is_bad_handle_with_nothing_to_close() {
    let mut table = table();
    let failed = table
        .replace(Handle::new(0, 3).unwrap(), Rights::NONE)
        .unwrap_err();
    assert_eq!(failed.status, Status::BadHandle);
    assert!(failed.closed.is_none());
}

#[test]
fn remove_needs_no_right_and_a_second_close_is_bad_handle() {
    let mut table = table();
    let object = plain(1);
    let handle = table.insert(object.clone(), Rights::NONE).unwrap();
    let transit = table.remove(handle).unwrap();
    assert_eq!(count(&object), 1);
    assert_eq!(table.remove(handle).err(), Some(Status::BadHandle));
    close(transit, &mut Stack::default());
    assert_eq!(count(&object), 0);
}

#[test]
fn zero_handles_fires_once_on_the_last_close_and_outside_the_table_lock() {
    let table = tab();
    let object = plain(7);
    let first = table
        .with(|t| t.insert(object.clone(), Rights::VMO))
        .unwrap();
    let second = table
        .with(|t| t.duplicate(first, Rights::DUPLICATE))
        .unwrap();
    close_handle(&table, first).unwrap();
    let plain_ref = object.plain_ref().unwrap();
    assert_eq!(
        plain_ref
            .zero_events
            .load(core::sync::atomic::Ordering::Relaxed),
        0
    );
    close_handle(&table, second).unwrap();
    assert_eq!(
        plain_ref
            .zero_events
            .load(core::sync::atomic::Ordering::Relaxed),
        1
    );
    assert_eq!(lock_depth(), 0);
}

#[test]
#[should_panic(expected = "handle fechado a mais")]
fn closing_below_zero_is_a_loud_bug() {
    let object = plain(1);
    object.handles().release();
}

#[test]
fn the_table_holds_4096_handles_and_the_next_is_no_memory() {
    let mut table = table();
    let mut handles = Vec::new();
    for id in 0..MAX_HANDLES {
        let rights = if id == 0 {
            Rights::DUPLICATE
        } else {
            Rights::NONE
        };
        handles.push(insert(&mut table, id, rights));
    }
    let extra = plain(9999);
    let refused = table.insert(extra.clone(), Rights::NONE).unwrap_err();
    assert_eq!(count(&refused), 0);
    assert_eq!(table.reserve().err(), Some(Status::NoMemory));
    assert_eq!(
        table.duplicate(handles[0], Rights::NONE),
        Err(Status::NoMemory)
    );
    close_one(&mut table, handles[10]);
    assert!(table.insert(extra, Rights::NONE).is_ok());
    assert_eq!(table.len(), MAX_HANDLES);
    assert_eq!(table.slot_count(), MAX_HANDLES);
}

#[test]
fn a_reservation_counts_against_the_cap() {
    let mut table = HandleTable::with_limits(
        VecSlots::default(),
        Limits {
            max_handles: 2,
            ..Limits::ABI
        },
    );
    let kept = insert(&mut table, 1, Rights::NONE);
    let reservation = table.reserve().unwrap();
    assert_eq!(table.reserve().err(), Some(Status::NoMemory));
    table.cancel(reservation);
    assert!(table.reserve().is_ok());
    assert!(table.get(kept, Rights::NONE).is_ok());
}

#[test]
fn storage_that_runs_out_is_no_memory_and_needs_no_allocator() {
    let mut table: HandleTable<TestObj, FixedSlots<TestObj, 2>> =
        HandleTable::new(FixedSlots::new());
    let a = table.insert(plain(1), Rights::NONE).unwrap();
    let b = table.insert(plain(2), Rights::NONE).unwrap();
    assert_eq!(table.reserve().err(), Some(Status::NoMemory));
    close_one_fixed(&mut table, a);
    assert!(table.insert(plain(3), Rights::NONE).is_ok());
    assert!(table.get(b, Rights::NONE).is_ok());
}

fn close_one_fixed(table: &mut HandleTable<TestObj, FixedSlots<TestObj, 2>>, handle: Handle) {
    let transit = table.remove(handle).unwrap();
    close(transit, &mut Stack::default());
}

#[test]
fn the_free_list_is_fifo_and_the_table_only_grows_when_it_is_empty() {
    let mut table = table();
    let a = insert(&mut table, 1, Rights::NONE);
    let b = insert(&mut table, 2, Rights::NONE);
    let c = insert(&mut table, 3, Rights::NONE);
    assert_eq!((a.index(), b.index(), c.index()), (0, 1, 2));
    close_one(&mut table, b);
    close_one(&mut table, a);
    let first = insert(&mut table, 4, Rights::NONE);
    let second = insert(&mut table, 5, Rights::NONE);
    assert_eq!((first.index(), second.index()), (1, 0));
    let third = insert(&mut table, 6, Rights::NONE);
    assert_eq!(third.index(), 3);
    assert_eq!(table.slot_count(), 4);
}

#[test]
fn reuse_raises_the_generation_and_the_old_value_stays_dead() {
    let mut table = table();
    let first = insert(&mut table, 1, Rights::NONE);
    close_one(&mut table, first);
    let second = insert(&mut table, 2, Rights::NONE);
    assert_eq!(second.index(), first.index());
    assert_eq!(second.generation(), first.generation() + 1);
    assert_eq!(
        table.get(first, Rights::NONE).err(),
        Some(Status::BadHandle)
    );
    assert!(table.get(second, Rights::NONE).is_ok());
}

#[test]
fn a_recycled_slot_retires_at_the_generation_budget_and_no_value_returns() {
    let limits = Limits {
        max_handles: 8,
        max_generation: 5,
    };
    let mut table = HandleTable::with_limits(VecSlots::default(), limits);
    let mut seen = std::collections::HashSet::new();
    let mut last = None;
    for round in 0..5 {
        let handle = insert(&mut table, round, Rights::NONE);
        assert!(seen.insert(handle.raw()), "valor repetido");
        assert_eq!(handle.index(), 0, "rodada {round}");
        assert_eq!(handle.generation(), round as u64);
        close_one(&mut table, handle);
        last = Some(handle);
    }
    assert_eq!(table.retired(), 1);
    let fresh = insert(&mut table, 100, Rights::NONE);
    assert_eq!(fresh.index(), 1);
    assert_eq!(fresh.generation(), 0);
    assert!(seen.insert(fresh.raw()));
    assert_eq!(
        table.get(last.unwrap(), Rights::NONE).err(),
        Some(Status::BadHandle)
    );
    assert_eq!(table.slot_count(), 2);
}

#[test]
fn at_the_real_48_bit_width_the_last_usable_generation_is_two_to_the_48_minus_two() {
    let mut table = table();
    let first = insert(&mut table, 1, Rights::NONE);
    close_one(&mut table, first);
    table.set_generation(0, Handle::MAX_GENERATION - 2);
    let near = insert(&mut table, 2, Rights::NONE);
    assert_eq!(near.generation(), Handle::MAX_GENERATION - 2);
    close_one(&mut table, near);
    let last = insert(&mut table, 3, Rights::NONE);
    assert_eq!(last.index(), 0);
    assert_eq!(last.generation(), Handle::MAX_GENERATION - 1);
    assert_eq!(Handle::from_raw(last.raw()), Some(last));
    close_one(&mut table, last);
    assert_eq!(table.retired(), 1);
    let after = insert(&mut table, 4, Rights::NONE);
    assert_eq!(after.index(), 1);
    assert_eq!(table.get(last, Rights::NONE).err(), Some(Status::BadHandle));
    assert_eq!(table.get(near, Rights::NONE).err(), Some(Status::BadHandle));
}

#[test]
fn the_abi_limits_are_the_documented_ones() {
    assert_eq!(Limits::ABI.max_handles, 4096);
    assert_eq!(Limits::ABI.max_generation, (1 << 48) - 1);
}

#[test]
fn a_reserved_value_is_not_valid_until_commit() {
    let mut table = table();
    let reservation = table.reserve().unwrap();
    let value = reservation.handle();
    assert_eq!(
        table.get(value, Rights::NONE).err(),
        Some(Status::BadHandle)
    );
    assert_eq!(table.remove(value).err(), Some(Status::BadHandle));
    assert_eq!((table.len(), table.reserved()), (0, 1));
    let object = plain(1);
    let handle = table.commit(reservation, object.clone(), Rights::READ);
    assert_eq!(handle, value);
    assert_eq!(count(&object), 1);
    assert_eq!((table.len(), table.reserved()), (1, 0));
    assert!(table.get(value, Rights::READ).is_ok());
}

#[test]
fn a_cancelled_reservation_leaves_no_trace_and_its_value_never_comes_back() {
    let mut table = table();
    let kept = insert(&mut table, 1, Rights::NONE);
    let mut cancelled = std::collections::HashSet::new();
    for _ in 0..50 {
        let reservation = table.reserve().unwrap();
        let value = reservation.handle();
        assert!(cancelled.insert(value.raw()), "valor reservado voltou");
        table.cancel(reservation);
        assert_eq!((table.len(), table.reserved()), (1, 0));
        assert_eq!(
            table.get(value, Rights::NONE).err(),
            Some(Status::BadHandle)
        );
    }
    for round in 0..50 {
        let handle = insert(&mut table, round, Rights::NONE);
        assert!(
            !cancelled.contains(&handle.raw()),
            "valor cancelado reapareceu"
        );
        close_one(&mut table, handle);
    }
    assert!(table.get(kept, Rights::NONE).is_ok());
}

#[test]
fn commit_moved_does_not_touch_the_count() {
    let mut table = table();
    let object = plain(1);
    let handle = table.insert(object.clone(), Rights::READ).unwrap();
    let transit = table.remove(handle).unwrap();
    let reservation = table.reserve().unwrap();
    let again = table.commit_moved(reservation, transit);
    assert_eq!(count(&object), 1);
    assert_eq!(table.get(again, Rights::READ).unwrap().rights, Rights::READ);
}

fn array(handles: &[(Handle, Rights)]) -> Vec<(Handle, Rights)> {
    handles.to_vec()
}

#[test]
fn take_array_removes_all_on_success_and_keeps_the_count() {
    let mut table = table();
    let a = plain(1);
    let b = plain(2);
    let ha = table.insert(a.clone(), Rights::TRANSFER).unwrap();
    let hb = table
        .insert(b.clone(), Rights::TRANSFER.union(Rights::READ))
        .unwrap();
    let taken = table
        .take_array(
            &array(&[(ha, Rights::TRANSFER), (hb, Rights::TRANSFER)]),
            None,
        )
        .unwrap();
    assert_eq!(taken.len(), 2);
    assert!(table.is_empty());
    assert_eq!((count(&a), count(&b)), (1, 1));
    taken.close_all(&mut Stack::default());
    assert_eq!((count(&a), count(&b)), (0, 0));
}

#[test]
fn take_array_refusals_still_consume_every_existing_handle() {
    let cases: [(&str, Status); 3] = [
        ("repetido", Status::BadHandle),
        ("inexistente", Status::BadHandle),
        ("sem direito", Status::AccessDenied),
    ];
    for (name, expected) in cases {
        let mut table = table();
        let a = plain(1);
        let b = plain(2);
        let ha = table.insert(a.clone(), Rights::TRANSFER).unwrap();
        let hb = table.insert(b.clone(), Rights::READ).unwrap();
        let items = match name {
            "repetido" => vec![(ha, Rights::TRANSFER), (ha, Rights::TRANSFER)],
            "inexistente" => vec![
                (ha, Rights::TRANSFER),
                (Handle::new(0, 50).unwrap(), Rights::NONE),
            ],
            _ => vec![(ha, Rights::TRANSFER), (hb, Rights::TRANSFER)],
        };
        let failed = table.take_array(&items, None).unwrap_err();
        assert_eq!(failed.status, expected, "{name}");
        failed.taken.close_all(&mut Stack::default());
        assert_eq!(count(&a), 0, "{name}");
        assert!(table.get(ha, Rights::NONE).is_err(), "{name}");
        if name == "sem direito" {
            assert_eq!(count(&b), 0, "{name}");
        } else {
            assert_eq!(count(&b), 1, "{name}");
        }
    }
}

#[test]
fn take_array_with_more_than_eight_is_invalid_args_and_takes_nothing() {
    let mut table = table();
    let handles: Vec<_> = (0..9)
        .map(|id| insert(&mut table, id, Rights::TRANSFER))
        .collect();
    let items: Vec<_> = handles.iter().map(|&h| (h, Rights::TRANSFER)).collect();
    let failed = table.take_array(&items, None).unwrap_err();
    assert_eq!(failed.status, Status::InvalidArgs);
    assert!(failed.taken.is_empty());
    assert_eq!(table.len(), 9);
}

#[test]
fn drain_hands_every_live_handle_to_the_worklist() {
    let mut table = table();
    let objects: Vec<_> = (0..5).map(plain).collect();
    for object in &objects {
        table.insert(object.clone(), Rights::NONE).unwrap();
    }
    let reservation = table.reserve().unwrap();
    let mut work = Stack::default();
    table.drain(&mut work);
    assert!(table.is_empty());
    assert_eq!(work.0.len(), 5);
    close_pending(&mut work);
    assert!(objects.iter().all(|object| count(object) == 0));
    table.cancel(reservation);
}
