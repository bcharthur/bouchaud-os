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

#[test]
fn le_fil_hid_garde_la_milliseconde_seulement_quand_un_hid_est_servi() {
    assert_eq!(politique::ticks_apres_tour_hid(true), 1);
    assert_eq!(politique::ticks_apres_tour_hid(false), politique::PAUSE_HID_OISIF_TICKS);
    // La pause n'ajoute pas plus qu'un huitieme a la periode de balayage
    // des branchements (200 ms), et reste bien au-dessus du tick.
    assert!(politique::PAUSE_HID_OISIF_TICKS > 1 && politique::PAUSE_HID_OISIF_TICKS * 8 <= 200);
}
