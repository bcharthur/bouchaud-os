extern crate alloc;

#[path = "../../src/kernel/native/abi/types.rs"]
mod types;
#[path = "../../src/kernel/native/abi/numbers.rs"]
mod numbers;
#[path = "../../src/kernel/native/abi/wire.rs"]
mod wire;

use types::{HandleId, Rights};

#[test]
fn handle_generation_prevents_aba() {
    let first = HandleId::new(7, 1);
    let second = HandleId::new(7, 2);
    assert_ne!(first, second);
    assert_eq!(first.slot(), second.slot());
    assert_ne!(first.generation(), second.generation());
    assert!(first.raw() <= i64::MAX as u64);
    assert!(second.raw() <= i64::MAX as u64);
}

#[test]
fn rights_never_grow_on_duplication() {
    let parent = Rights::READ | Rights::WRITE | Rights::DUP;
    assert!(Rights::READ.subset_of(parent));
    assert!((Rights::READ | Rights::WRITE).subset_of(parent));
    assert!(!Rights::TRANSFER.subset_of(parent));
}

#[test]
fn native_namespace_is_disjoint_from_linux_range() {
    assert!(numbers::is_native(numbers::VERSION));
    assert!(numbers::is_native(numbers::SHM_WRITE));
    for linux in 0..=512 {
        assert!(!numbers::is_native(linux));
    }
}

#[test]
fn wire_structures_are_stable() {
    assert_eq!(wire::RecvMeta::BYTE_LEN, 16);
    assert_eq!(wire::HandleInfo::BYTE_LEN, 16);
    assert_eq!(wire::WaitEvent::BYTE_LEN, 16);
}
