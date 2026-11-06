//! Regression for issue #262 and PR #265: both regular USB BOT and
//! the blackbox BOT path must correlate completions to the submitted TRB.
use std::fs;
fn read(path: &str) -> String { fs::read_to_string(path).expect(path) }
#[test]
fn regular_bot_uses_exact_trb_pointer() {
    let s = read("src/drivers/usb/xhci_active.rs");
    let f = s.split("fn bulk_transfert(").nth(1).expect("bulk_transfert absent");
    let f = f.split("/// Debloque un point").next().unwrap();
    assert!(f.contains("let trb_phys = ring_push("));
    assert!(f.contains("Some(trb_phys)"));
    assert!(s.contains("event.parameter & !0xf == p & !0xf"));
    assert!(s.contains("BOUCHAUD_USB_BULK_TRB_PERIME"));
}
#[test]
fn blackbox_bot_never_accepts_unfiltered_transfer_event() {
    let s = read("src/drivers/usb/blackbox_storage.rs");
    let f = s.split("fn blackbox_bulk_transfer(").nth(1).unwrap();
    let f = f.split("/// Une commande BOT").next().unwrap();
    assert!(f.contains("let trb_phys = ring_push("));
    assert!(f.contains("fence(Ordering::SeqCst)"));
    assert!(f.contains("wait_event_budget("));
    assert!(f.contains("Some(trb_phys)"));
    assert!(!f.contains("wait_event_budget_unfiltered("));
}
#[test]
fn trb_pointer_matching_rejects_stale_events() {
    let expected = 0x1_2300_20u64;
    assert_eq!(expected & !0xf, (expected | 0x5) & !0xf);
    assert_ne!(expected & !0xf, (expected + 16) & !0xf);
    assert_ne!(expected & !0xf, 0x1_2400_20u64 & !0xf);
}
