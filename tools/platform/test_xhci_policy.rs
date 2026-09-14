#[path = "../../src/drivers/usb/xhci_policy.rs"]
mod politique;

#[test]
fn scratchpads_champs_asymetriques() {
    assert_eq!(politique::max_scratchpads(2 << 27), 2);
    assert_eq!(politique::max_scratchpads(2 << 21), 64);
    assert_eq!(politique::max_scratchpads((3 << 21) | (7 << 27)), 103);
    assert_eq!(politique::max_scratchpads(u32::MAX), 1023);
    assert_eq!(politique::max_scratchpads(1 << 26), 0);
}

#[test]
fn aucun_rearmement_sur_contexte_running_ou_disabled() {
    for etat in [0, 1, 5, 6, 7] {
        assert!(!politique::recuperable(etat));
    }
    for etat in [2, 3, 4] {
        assert!(politique::recuperable(etat));
    }
}
