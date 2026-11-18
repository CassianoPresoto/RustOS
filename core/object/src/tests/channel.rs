//! A fila do canal e `channel_write`/`channel_read`: um teste por regra de recusa, a destruição
//! por contagem de handles e as lacunas L1 e L2 (SDD 17).

use std::sync::atomic::Ordering;

use rustos_hal::abi::{
    CHANNEL_MAX_BYTES, CHANNEL_MAX_HANDLES, CHANNEL_QUEUE_LEN, Handle, Rights, Status, signals,
};

use super::support::{
    Stack, Tab, TestObj, close_handle, count, handle_of, lock_depth, new_channel, read, tab, write,
};
use crate::{Channel, Lock, Message, ObjectKind, Side, Transits, channel_read, close};

struct Pair {
    p: Tab,
    q: Tab,
    /// A ponta A, na tabela de `p`.
    ha: Handle,
    /// A ponta B, na tabela de `q`.
    hb: Handle,
    a: TestObj,
    b: TestObj,
}

fn pair() -> Pair {
    let (a, b) = new_channel();
    let (p, q) = (tab(), tab());
    let ha = handle_of(&p, &a, Rights::CHANNEL);
    let hb = handle_of(&q, &b, Rights::CHANNEL);
    Pair { p, q, ha, hb, a, b }
}

fn plain(id: usize) -> TestObj {
    TestObj::plain(ObjectKind::Vmo, id)
}

fn with_chan<R>(object: &TestObj, f: impl FnOnce(&mut Channel<TestObj>) -> R) -> R {
    use crate::Object;
    object.with_channel(f).unwrap()
}

fn signals_of(object: &TestObj, side: Side) -> u32 {
    with_chan(object, |chan| chan.signals(side))
}

fn dead(object: &TestObj, side: Side) -> bool {
    with_chan(object, |chan| chan.is_dead(side))
}

fn live_handles(table: &Tab) -> usize {
    table.with(|t| t.len())
}

fn gone(table: &Tab, handle: Handle) -> bool {
    table.with(|t| t.get(handle, Rights::NONE).is_err())
}

#[test]
fn a_message_with_four_handles_goes_from_one_table_to_the_other() {
    let pair = pair();
    let objects: Vec<_> = (0..4).map(plain).collect();
    let rights = [
        Rights::READ,
        Rights::TRANSFER.union(Rights::WRITE),
        Rights::VMO,
        Rights::NONE.union(Rights::TRANSFER),
    ];
    let sent: Vec<_> = objects
        .iter()
        .zip(rights)
        .map(|(object, rights)| handle_of(&pair.p, object, rights.union(Rights::TRANSFER)))
        .collect();
    write(&pair.p, pair.ha, b"hello", &sent).unwrap();
    assert!(sent.iter().all(|&handle| gone(&pair.p, handle)));
    assert_eq!(live_handles(&pair.p), 1);
    assert!(objects.iter().all(|object| count(object) == 1));
    assert_eq!(
        signals_of(&pair.a, Side::B) & signals::READABLE,
        signals::READABLE
    );

    let (bytes, received) = read(&pair.q, pair.hb, 1024, 8).unwrap();
    assert_eq!(bytes, b"hello");
    assert_eq!(received.len(), 4);
    for ((handle, object), rights) in received.iter().zip(&objects).zip(rights) {
        let granted = pair.q.with(|t| {
            t.get(*handle, Rights::NONE)
                .map(|g| (g.rights, g.object.clone()))
        });
        let (granted_rights, got) = granted.unwrap();
        assert_eq!(granted_rights, rights.union(Rights::TRANSFER));
        assert!(std::sync::Arc::ptr_eq(
            got.plain_ref().unwrap(),
            object.plain_ref().unwrap()
        ));
        assert_eq!(count(object), 1);
    }
    assert_eq!(live_handles(&pair.q), 5);
    assert_eq!(signals_of(&pair.a, Side::B) & signals::READABLE, 0);
}

#[test]
fn bytes_and_handles_over_the_limits_are_invalid_args_and_consume_nothing() {
    let pair = pair();
    let object = plain(1);
    let handle = handle_of(&pair.p, &object, Rights::TRANSFER);
    let big = vec![0_u8; CHANNEL_MAX_BYTES + 1];
    assert_eq!(
        write(&pair.p, pair.ha, &big, &[handle]),
        Err(Status::InvalidArgs)
    );
    let nine = vec![handle; CHANNEL_MAX_HANDLES + 1];
    assert_eq!(
        write(&pair.p, pair.ha, b"", &nine),
        Err(Status::InvalidArgs)
    );
    assert!(!gone(&pair.p, handle));
    assert_eq!(count(&object), 1);
    assert_eq!(with_chan(&pair.b, |chan| chan.queued(Side::B)), 0);
}

#[test]
fn the_limits_themselves_are_accepted() {
    let pair = pair();
    let handles: Vec<_> = (0..CHANNEL_MAX_HANDLES)
        .map(|id| handle_of(&pair.p, &plain(id), Rights::TRANSFER))
        .collect();
    let bytes = vec![7_u8; CHANNEL_MAX_BYTES];
    write(&pair.p, pair.ha, &bytes, &handles).unwrap();
    let (got, received) = read(&pair.q, pair.hb, CHANNEL_MAX_BYTES, CHANNEL_MAX_HANDLES).unwrap();
    assert_eq!(
        (got.len(), received.len()),
        (CHANNEL_MAX_BYTES, CHANNEL_MAX_HANDLES)
    );
    write(&pair.p, pair.ha, b"", &[]).unwrap();
    let (empty, none) = read(&pair.q, pair.hb, 0, 0).unwrap();
    assert!(empty.is_empty() && none.is_empty());
}

#[test]
fn a_full_queue_is_should_wait_and_the_handle_stays_in_the_table() {
    let pair = pair();
    for _ in 0..CHANNEL_QUEUE_LEN {
        write(&pair.p, pair.ha, b"x", &[]).unwrap();
    }
    assert_eq!(signals_of(&pair.a, Side::A) & signals::WRITABLE, 0);
    let object = plain(1);
    let handle = handle_of(&pair.p, &object, Rights::TRANSFER);
    assert_eq!(
        write(&pair.p, pair.ha, b"y", &[handle]),
        Err(Status::ShouldWait)
    );
    assert!(!gone(&pair.p, handle));
    assert_eq!(count(&object), 1);
    read(&pair.q, pair.hb, 8, 0).unwrap();
    assert_eq!(
        signals_of(&pair.a, Side::A) & signals::WRITABLE,
        signals::WRITABLE
    );
    write(&pair.p, pair.ha, b"y", &[handle]).unwrap();
    assert!(gone(&pair.p, handle));
    assert_eq!(count(&object), 1);
}

#[test]
fn should_wait_hides_an_array_error_until_the_retry() {
    let pair = pair();
    for _ in 0..CHANNEL_QUEUE_LEN {
        write(&pair.p, pair.ha, b"x", &[]).unwrap();
    }
    let object = plain(1);
    let no_transfer = handle_of(&pair.p, &object, Rights::READ);
    assert_eq!(
        write(&pair.p, pair.ha, b"", &[no_transfer]),
        Err(Status::ShouldWait)
    );
    assert!(!gone(&pair.p, no_transfer));
    read(&pair.q, pair.hb, 8, 0).unwrap();
    assert_eq!(
        write(&pair.p, pair.ha, b"", &[no_transfer]),
        Err(Status::AccessDenied)
    );
    assert!(gone(&pair.p, no_transfer));
}

fn refusal(setup: impl FnOnce(&Pair) -> (Handle, Vec<Handle>, Vec<TestObj>), expected: Status) {
    let pair = pair();
    let (channel, handles, objects) = setup(&pair);
    assert_eq!(write(&pair.p, channel, b"m", &handles), Err(expected));
    for handle in &handles {
        assert!(
            gone(&pair.p, *handle) || *handle == channel,
            "{handle:?} sobrou"
        );
    }
    for object in &objects {
        assert_eq!(count(object), 0, "handle em trânsito vazou");
    }
    assert_eq!(with_chan(&pair.b, |chan| chan.queued(Side::B)), 0);
    assert_eq!(
        with_chan(&pair.a, |chan| chan.signals(Side::A)) & signals::WRITABLE,
        signals::WRITABLE
    );
}

fn two(pair: &Pair) -> (Vec<Handle>, Vec<TestObj>) {
    let objects = vec![plain(1), plain(2)];
    let handles = objects
        .iter()
        .map(|object| handle_of(&pair.p, object, Rights::TRANSFER))
        .collect();
    (handles, objects)
}

#[test]
fn a_repeated_handle_is_bad_handle_and_consumes_the_array() {
    refusal(
        |pair| {
            let (handles, objects) = two(pair);
            (pair.ha, vec![handles[0], handles[1], handles[0]], objects)
        },
        Status::BadHandle,
    );
}

#[test]
fn an_unknown_handle_in_the_array_is_bad_handle_and_consumes_the_others() {
    refusal(
        |pair| {
            let (handles, objects) = two(pair);
            let unknown = Handle::new(0, 40).unwrap();
            (pair.ha, vec![handles[0], unknown, handles[1]], objects)
        },
        Status::BadHandle,
    );
}

#[test]
fn a_handle_without_transfer_is_access_denied_and_consumes_the_array() {
    refusal(
        |pair| {
            let (handles, mut objects) = two(pair);
            let loose = plain(3);
            let no_transfer = handle_of(&pair.p, &loose, Rights::READ);
            objects.push(loose);
            (pair.ha, vec![handles[0], no_transfer, handles[1]], objects)
        },
        Status::AccessDenied,
    );
}

#[test]
fn the_own_end_by_any_handle_is_not_supported_and_consumes_the_array() {
    refusal(
        |pair| {
            let (handles, objects) = two(pair);
            (pair.ha, vec![handles[0], pair.ha, handles[1]], objects)
        },
        Status::NotSupported,
    );
    refusal(
        |pair| {
            let (handles, objects) = two(pair);
            let again = pair
                .p
                .with(|t| t.duplicate(pair.ha, Rights::TRANSFER))
                .unwrap();
            (pair.ha, vec![handles[0], again], vec![objects[0].clone()])
        },
        Status::NotSupported,
    );
}

#[test]
fn a_bad_channel_handle_consumes_the_array_that_exists() {
    refusal(
        |pair| {
            let (handles, objects) = two(pair);
            (Handle::new(0, 41).unwrap(), handles, objects)
        },
        Status::BadHandle,
    );
}

#[test]
fn a_channel_without_write_is_access_denied_and_consumes_the_array() {
    refusal(
        |pair| {
            let (handles, objects) = two(pair);
            let read_only = pair.p.with(|t| t.duplicate(pair.ha, Rights::READ)).unwrap();
            (read_only, handles, objects)
        },
        Status::AccessDenied,
    );
}

#[test]
fn a_handle_that_is_not_a_channel_is_wrong_type_and_consumes_the_array() {
    refusal(
        |pair| {
            let (handles, objects) = two(pair);
            let not_channel = handle_of(&pair.p, &plain(9), Rights::WRITE);
            (not_channel, handles, objects)
        },
        Status::WrongType,
    );
}

#[test]
fn writing_to_a_closed_peer_is_peer_closed_and_consumes_the_array() {
    let pair = pair();
    let (handles, objects) = two(&pair);
    close_handle(&pair.q, pair.hb).unwrap();
    assert!(dead(&pair.a, Side::B));
    assert_eq!(
        write(&pair.p, pair.ha, b"m", &handles),
        Err(Status::PeerClosed)
    );
    assert!(handles.iter().all(|&handle| gone(&pair.p, handle)));
    assert!(objects.iter().all(|object| count(object) == 0));
}

#[test]
fn the_peer_end_of_the_same_channel_may_travel() {
    let pair = pair();
    let b2 = pair
        .p
        .with(|t| t.insert(pair.b.clone(), Rights::CHANNEL))
        .unwrap();
    write(&pair.p, pair.ha, b"here", &[b2]).unwrap();
    assert!(gone(&pair.p, b2));
    let (_, received) = read(&pair.q, pair.hb, 16, 1).unwrap();
    assert_eq!(count(&pair.b), 2);
    assert_eq!(received.len(), 1);
}

#[test]
fn a_small_buffer_is_buffer_too_small_with_the_sizes_and_the_message_stays() {
    let pair = pair();
    let object = plain(1);
    let handle = handle_of(&pair.p, &object, Rights::TRANSFER);
    write(&pair.p, pair.ha, b"abcdef", &[handle]).unwrap();
    let failed = read(&pair.q, pair.hb, 5, 1).unwrap_err();
    assert_eq!(failed.status, Status::BufferTooSmall);
    assert_eq!((failed.actual.bytes, failed.actual.handles), (6, 1));
    let failed = read(&pair.q, pair.hb, 6, 0).unwrap_err();
    assert_eq!(
        (failed.status, failed.actual.handles),
        (Status::BufferTooSmall, 1)
    );
    assert_eq!(live_handles(&pair.q), 1);
    assert_eq!(with_chan(&pair.b, |chan| chan.queued(Side::B)), 1);
    assert_eq!(read(&pair.q, pair.hb, 6, 1).unwrap().0, b"abcdef");
}

#[test]
fn reading_an_empty_queue_is_should_wait_and_after_the_peer_dies_peer_closed() {
    let pair = pair();
    assert_eq!(
        read(&pair.q, pair.hb, 8, 8).unwrap_err().status,
        Status::ShouldWait
    );
    close_handle(&pair.p, pair.ha).unwrap();
    assert_eq!(
        read(&pair.q, pair.hb, 8, 8).unwrap_err().status,
        Status::PeerClosed
    );
    assert_eq!(signals_of(&pair.b, Side::B), signals::PEER_CLOSED);
}

#[test]
fn who_writes_and_closes_is_still_read() {
    let pair = pair();
    write(&pair.p, pair.ha, b"last words", &[]).unwrap();
    close_handle(&pair.p, pair.ha).unwrap();
    assert_eq!(
        signals_of(&pair.b, Side::B),
        signals::READABLE | signals::PEER_CLOSED
    );
    assert_eq!(read(&pair.q, pair.hb, 64, 0).unwrap().0, b"last words");
    assert_eq!(
        read(&pair.q, pair.hb, 64, 0).unwrap_err().status,
        Status::PeerClosed
    );
}

#[test]
fn read_refuses_a_missing_right_a_wrong_type_and_an_unknown_handle() {
    let pair = pair();
    write(&pair.p, pair.ha, b"m", &[]).unwrap();
    let write_only = pair
        .q
        .with(|t| t.duplicate(pair.hb, Rights::WRITE))
        .unwrap();
    assert_eq!(
        read(&pair.q, write_only, 8, 8).unwrap_err().status,
        Status::AccessDenied
    );
    let vmo = handle_of(&pair.q, &plain(1), Rights::READ);
    assert_eq!(
        read(&pair.q, vmo, 8, 8).unwrap_err().status,
        Status::WrongType
    );
    let unknown = Handle::new(0, 77).unwrap();
    assert_eq!(
        read(&pair.q, unknown, 8, 8).unwrap_err().status,
        Status::BadHandle
    );
    assert_eq!(with_chan(&pair.b, |chan| chan.queued(Side::B)), 1);
}

#[test]
fn a_failed_copy_to_the_user_leaves_the_message_and_the_table_intact() {
    let pair = pair();
    let object = plain(1);
    let handle = handle_of(&pair.p, &object, Rights::TRANSFER);
    write(&pair.p, pair.ha, b"keep", &[handle]).unwrap();
    let before = live_handles(&pair.q);
    let failed = channel_read(&pair.q, pair.hb, 64, 8, |_, _| false).unwrap_err();
    assert_eq!(failed.status, Status::OutOfRange);
    assert_eq!(live_handles(&pair.q), before);
    assert_eq!(pair.q.with(|t| t.reserved()), 0);
    assert_eq!(count(&object), 1);
    assert_eq!(read(&pair.q, pair.hb, 64, 8).unwrap().0, b"keep");
}

#[test]
fn a_read_that_finds_no_slot_is_no_memory_and_the_message_stays() {
    use super::support::{Table, VecSlots};
    use crate::{HandleTable, Limits};
    let pair = pair();
    let handle = handle_of(&pair.p, &plain(1), Rights::TRANSFER);
    write(&pair.p, pair.ha, b"m", &[handle]).unwrap();
    let small: Table = HandleTable::with_limits(
        VecSlots::default(),
        Limits {
            max_handles: 1,
            ..Limits::ABI
        },
    );
    let crowded = super::support::Locked::new(small);
    let hb = handle_of(&crowded, &pair.b, Rights::CHANNEL);
    let failed = read(&crowded, hb, 8, 8).unwrap_err();
    assert_eq!(failed.status, Status::NoMemory);
    assert_eq!(with_chan(&pair.b, |chan| chan.queued(Side::B)), 1);
    assert_eq!(crowded.with(|t| (t.len(), t.reserved())), (1, 0));
}

#[test]
fn a_competing_read_between_the_copy_and_the_commit_makes_the_read_repeat() {
    let pair = pair();
    let first = plain(1);
    let second = plain(2);
    write(
        &pair.p,
        pair.ha,
        b"one",
        &[handle_of(&pair.p, &first, Rights::TRANSFER)],
    )
    .unwrap();
    write(
        &pair.p,
        pair.ha,
        b"two",
        &[handle_of(&pair.p, &second, Rights::TRANSFER)],
    )
    .unwrap();
    let mut attempts = Vec::new();
    let mut stolen = None;
    let outer = channel_read(&pair.q, pair.hb, 8, 8, |bytes, handles| {
        attempts.push((bytes.to_vec(), handles.to_vec()));
        if stolen.is_none() {
            stolen = Some(read(&pair.q, pair.hb, 8, 8).unwrap());
        }
        true
    })
    .unwrap();
    assert_eq!(outer.handles, 1);
    let stolen = stolen.unwrap();
    assert_eq!(stolen.0, b"one");
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0].0, b"one");
    assert_eq!(attempts[1].0, b"two");
    // O valor reservado e solto na primeira tentativa não reaparece em handle nenhum.
    let released = attempts[0].1[0];
    let all: Vec<_> = std::iter::once(&attempts[1].1[0])
        .chain(stolen.1.iter())
        .collect();
    assert!(!all.contains(&&released), "o valor solto voltou");
    assert_eq!(pair.q.with(|t| (t.len(), t.reserved())), (3, 0));
    assert_eq!(count(&first), 1);
    assert_eq!(count(&second), 1);
    assert_eq!(with_chan(&pair.b, |chan| chan.queued(Side::B)), 0);
}

#[test]
fn the_peer_dying_between_reserve_and_commit_returns_the_message_for_closing() {
    let pair = pair();
    let object = plain(1);
    let handle = handle_of(&pair.p, &object, Rights::TRANSFER);
    let ticket = with_chan(&pair.a, |chan| chan.reserve(Side::A)).unwrap();
    let taken = pair.p.with(|t| t.remove_existing(&[handle]));
    close_handle(&pair.q, pair.hb).unwrap();
    let message = Message::new(std::sync::Arc::from(&b"late"[..]), taken).unwrap();
    let mut work = Stack::default();
    let returned = with_chan(&pair.a, |chan| chan.commit(ticket, message, &mut work));
    let (_, handles) = returned.unwrap_err().into_parts();
    handles.close_all(&mut work);
    assert_eq!(count(&object), 0);
    assert_eq!(with_chan(&pair.a, |chan| chan.queued(Side::B)), 0);
}

#[test]
fn a_reservation_holds_the_slot_and_cancel_gives_it_back() {
    let pair = pair();
    let mut tickets = Vec::new();
    for _ in 0..CHANNEL_QUEUE_LEN {
        tickets.push(with_chan(&pair.a, |chan| chan.reserve(Side::A)).unwrap());
    }
    assert_eq!(
        with_chan(&pair.a, |chan| chan.reserve(Side::A)).err(),
        Some(Status::ShouldWait)
    );
    assert_eq!(write(&pair.p, pair.ha, b"x", &[]), Err(Status::ShouldWait));
    for ticket in tickets {
        with_chan(&pair.a, |chan| chan.cancel(ticket));
    }
    write(&pair.p, pair.ha, b"x", &[]).unwrap();
}

#[test]
fn message_new_refuses_what_the_channel_cannot_carry() {
    let too_long: std::sync::Arc<[u8]> = vec![0; CHANNEL_MAX_BYTES + 1].into();
    let err = Message::<TestObj>::new(too_long, Transits::new()).unwrap_err();
    assert_eq!(err.0, Status::InvalidArgs);
    let ok: std::sync::Arc<[u8]> = vec![0; CHANNEL_MAX_BYTES].into();
    assert!(Message::<TestObj>::new(ok, Transits::new()).is_ok());
}

#[test]
fn a_dead_end_drops_the_queue_and_closes_the_handles_inside() {
    let pair = pair();
    let objects: Vec<_> = (0..3).map(plain).collect();
    for object in &objects {
        let handle = handle_of(&pair.p, object, Rights::TRANSFER);
        write(&pair.p, pair.ha, b"m", &[handle]).unwrap();
    }
    assert!(objects.iter().all(|object| count(object) == 1));
    close_handle(&pair.q, pair.hb).unwrap();
    assert!(objects.iter().all(|object| count(object) == 0));
    assert!(dead(&pair.a, Side::B));
    assert_eq!(with_chan(&pair.a, |chan| chan.queued(Side::B)), 0);
    assert_eq!(
        signals_of(&pair.a, Side::A) & signals::PEER_CLOSED,
        signals::PEER_CLOSED
    );
    assert_eq!(
        pair.b
            .end_ref()
            .unwrap()
            .peer_closed_calls
            .load(Ordering::Relaxed),
        1
    );
    assert_eq!(lock_depth(), 0);
}

#[test]
fn the_cycle_writing_one_end_through_the_other_kills_it_on_the_write_then_close() {
    let (a, b) = new_channel();
    let both = tab();
    let ha = handle_of(&both, &a, Rights::CHANNEL);
    let hb = handle_of(&both, &b, Rights::CHANNEL);
    assert_eq!((count(&a), count(&b)), (1, 1));
    write(&both, ha, b"cycle", &[hb]).unwrap();
    assert!(dead(&a, Side::B), "B morre na própria escrita");
    assert_eq!(count(&b), 0);
    assert!(!dead(&a, Side::A));
    assert_eq!(with_chan(&a, |chan| chan.own_in_queue(Side::B)), 0);
    close_handle(&both, ha).unwrap();
    assert!(dead(&a, Side::A));
    assert_eq!((count(&a), count(&b)), (0, 0));
}

#[test]
fn the_cycle_closing_the_other_handle_first_also_ends_with_both_ends_dead() {
    let (a, b) = new_channel();
    let both = tab();
    let ha = handle_of(&both, &a, Rights::CHANNEL);
    let hb = handle_of(&both, &b, Rights::CHANNEL);
    let hb2 = both.with(|t| t.duplicate(hb, Rights::CHANNEL)).unwrap();
    close_handle(&both, hb).unwrap();
    assert!(!dead(&a, Side::B));
    write(&both, ha, b"cycle", &[hb2]).unwrap();
    assert!(dead(&a, Side::B));
    close_handle(&both, ha).unwrap();
    assert!(dead(&a, Side::A));
    assert_eq!((count(&a), count(&b)), (0, 0));
    assert_eq!(live_handles(&both), 0);
}

#[test]
fn the_peer_closed_before_the_write_ends_the_same_way() {
    let (a, b) = new_channel();
    let both = tab();
    let ha = handle_of(&both, &a, Rights::CHANNEL);
    let hb = handle_of(&both, &b, Rights::CHANNEL);
    close_handle(&both, hb).unwrap();
    assert!(dead(&a, Side::B));
    assert_eq!(write(&both, ha, b"x", &[]), Err(Status::PeerClosed));
    close_handle(&both, ha).unwrap();
    assert_eq!((count(&a), count(&b)), (0, 0));
}

#[test]
fn a_chain_of_channels_dies_in_a_loop_without_recursion() {
    let depth = 2000;
    let holder = tab();
    let mut ends = Vec::new();
    let mut handles = Vec::new();
    for _ in 0..depth {
        let (a, b) = new_channel();
        handles.push((
            handle_of(&holder, &a, Rights::CHANNEL),
            handle_of(&holder, &b, Rights::CHANNEL),
        ));
        ends.push((a, b));
    }
    for i in (1..depth).rev() {
        write(
            &holder,
            handles[i - 1].0,
            b"",
            &[handles[i].0, handles[i].1],
        )
        .unwrap();
    }
    assert_eq!(live_handles(&holder), 2);
    assert!(ends.iter().all(|(a, b)| count(a) == 1 && count(b) == 1));
    close_handle(&holder, handles[0].0).unwrap();
    assert!(
        !dead(&ends[0].0, Side::B),
        "a B_0 ainda tem o handle da tabela"
    );
    close_handle(&holder, handles[0].1).unwrap();
    assert!(ends.iter().all(|(a, b)| count(a) == 0 && count(b) == 0));
    assert!(
        ends.iter()
            .all(|(a, _)| dead(a, Side::A) && dead(a, Side::B))
    );
}

#[test]
fn closing_in_the_table_lock_never_happens_while_the_lock_is_held() {
    let pair = pair();
    let object = plain(1);
    let handle = handle_of(&pair.p, &object, Rights::TRANSFER);
    write(&pair.p, pair.ha, b"m", &[handle]).unwrap();
    let mut work = Stack::default();
    // Refaz o fim da vida da ponta B com a tabela de `q` observada: o gancho confere a
    // profundidade do lock em `on_zero_handles` e a trava do par em `on_end_died`.
    let transit = pair.q.with(|t| t.remove(pair.hb)).unwrap();
    assert_eq!(lock_depth(), 0);
    close(transit, &mut work);
    assert_eq!(count(&object), 0);
}
