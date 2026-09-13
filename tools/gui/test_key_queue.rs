extern crate alloc;
#[path = "../../src/gui/key_queue.rs"] mod key_queue;
use key_queue::KeyQueue;
#[test]
fn stalled_reader_preserves_press_release_order() {
    let mut q = KeyQueue::new();
    let mut press = [0; 20]; press[16] = 1;
    let release = [0; 20];
    assert!(q.push(press)); assert!(q.push(release));
    for _ in 0..100 { assert_eq!(q.front(), Some(press)); }
    q.delivered(); assert_eq!(q.front(), Some(release));
    q.delivered(); assert_eq!(q.front(), None);
}
#[test]
fn overflow_does_not_overwrite_pending_keys_and_drain_recovers() {
    let mut q = KeyQueue::new();
    for i in 0..256 { let mut k=[0;20]; k[0]=i as u8; assert!(q.push(k)); }
    assert!(!q.push([99;20]));
    for i in 0..256 { assert_eq!(q.front().unwrap()[0], i as u8); q.delivered(); }
    assert!(q.push([7;20])); assert_eq!(q.front(), Some([7;20]));
}
