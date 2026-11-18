//! Regras de direito: o subconjunto e o que cada operação exige, contra as tabelas do SDD 17.

use rustos_hal::abi::{Rights, Status, vmar_flags};

use crate::ObjectKind;
use crate::rights::{parse, reduce, require, required};

#[test]
fn reserved_bits_are_invalid_args() {
    assert_eq!(parse(0x7FF), Ok(Rights::ALL));
    assert_eq!(parse(1 << 11), Err(Status::InvalidArgs));
    assert_eq!(parse(u32::MAX), Err(Status::InvalidArgs));
    assert_eq!(parse(0), Ok(Rights::NONE));
}

#[test]
fn missing_right_is_access_denied() {
    assert_eq!(
        require(Rights::READ, Rights::WRITE),
        Err(Status::AccessDenied)
    );
    assert_eq!(
        require(Rights::READ, Rights::READ.union(Rights::WRITE)),
        Err(Status::AccessDenied)
    );
    assert_eq!(require(Rights::NONE, Rights::NONE), Ok(()));
    assert_eq!(require(Rights::CHANNEL, Rights::WRITE), Ok(()));
}

#[test]
fn reduce_accepts_only_a_subset() {
    assert_eq!(reduce(Rights::VMO, Rights::READ), Ok(Rights::READ));
    assert_eq!(reduce(Rights::VMO, Rights::VMO), Ok(Rights::VMO));
    assert_eq!(
        reduce(Rights::READ, Rights::WRITE),
        Err(Status::InvalidArgs)
    );
    assert_eq!(reduce(Rights::READ, Rights::VMO), Err(Status::InvalidArgs));
    assert_eq!(
        reduce(Rights::NONE, Rights::DUPLICATE),
        Err(Status::InvalidArgs)
    );
}

#[test]
fn each_operation_requires_what_the_sdd_table_says() {
    assert_eq!(required::HANDLE_CLOSE, Rights::NONE);
    assert_eq!(required::HANDLE_REPLACE, Rights::NONE);
    assert_eq!(required::HANDLE_DUPLICATE, Rights::DUPLICATE);
    assert_eq!(required::OBJECT_WAIT, Rights::WAIT);
    assert_eq!(required::DEBUG_WRITE, Rights::WRITE);
    assert_eq!(required::CHANNEL_WRITE, Rights::WRITE);
    assert_eq!(required::CHANNEL_READ, Rights::READ);
    assert_eq!(required::TRANSFER, Rights::TRANSFER);
    assert_eq!(required::PROCESS_CREATE_PARENT, Rights::MANAGE_PROCESS);
    assert_eq!(required::PROCESS_CREATE_BOOTSTRAP, Rights::TRANSFER);
    assert_eq!(
        required::PROCESS_CREATE_EVENTS,
        Rights::WRITE.union(Rights::TRANSFER)
    );
    assert_eq!(required::THREAD_CREATE, Rights::MANAGE_THREAD);
    assert_eq!(required::PROCESS_KILL, Rights::DESTROY);
    assert_eq!(required::VMAR_UNMAP, Rights::MAP);
}

#[test]
fn vmar_map_asks_map_and_each_permission_in_the_right_numbering() {
    let rw = required::vmar_map(vmar_flags::READ | vmar_flags::WRITE);
    assert_eq!(rw, Rights::MAP.union(Rights::READ).union(Rights::WRITE));
    assert_eq!(
        required::vmar_map(vmar_flags::EXECUTE | vmar_flags::FIXED),
        Rights::MAP.union(Rights::EXECUTE)
    );
    assert_eq!(required::vmar_map(0), Rights::MAP);
}

#[test]
fn rights_of_each_kind_are_the_sdd_table_and_the_initial_ones_fit_in() {
    let table = [
        (
            ObjectKind::Process,
            Rights::DUPLICATE
                .union(Rights::TRANSFER)
                .union(Rights::WAIT)
                .union(Rights::INSPECT)
                .union(Rights::DESTROY)
                .union(Rights::MANAGE_PROCESS)
                .union(Rights::MANAGE_THREAD),
        ),
        (
            ObjectKind::Thread,
            Rights::DUPLICATE
                .union(Rights::TRANSFER)
                .union(Rights::WAIT)
                .union(Rights::INSPECT),
        ),
        (
            ObjectKind::Channel,
            Rights::DUPLICATE
                .union(Rights::TRANSFER)
                .union(Rights::READ)
                .union(Rights::WRITE)
                .union(Rights::WAIT),
        ),
        (
            ObjectKind::Console,
            Rights::DUPLICATE
                .union(Rights::TRANSFER)
                .union(Rights::WRITE),
        ),
    ];
    for (kind, expected) in table {
        assert_eq!(kind.rights(), expected, "{kind:?}");
    }
    assert!(Rights::INITRAMFS.is_subset_of(ObjectKind::Vmo.rights()));
    assert!(Rights::VMAR.is_subset_of(ObjectKind::Vmar.rights()));
    assert!(!ObjectKind::Vmar.rights().contains(Rights::WAIT));
}
