//! Decodage des capacites xHCI, independant du materiel.

/// HCSPARAMS2 : Hi en 25:21, Lo en 31:27 (xHCI 1.2, section 5.3.4).
pub const fn max_scratchpads(hcs2: u32) -> usize {
    ((((hcs2 >> 21) & 0x1f) << 5) | ((hcs2 >> 27) & 0x1f)) as usize
}

/// Le contexte de sortie peut etre en retard pendant Running. Son pointeur
/// ne prouve donc jamais qu'un TD est perdu. Seuls ces etats autorisent une
/// reprise par Reset Endpoint / Set TR Dequeue Pointer.
pub const fn recuperable(etat: u32) -> bool {
    matches!(etat, 2 | 3 | 4) // Halted, Stopped, Error
}

/// P13_HID_SANS_PERIPHERIQUE_V1 : pause du fil `usb-hid` quand il n'a aucun
/// HID a lire (aucun controleur actif, ou aucun clavier/souris enumere), en
/// ticks. Le balayage des branchements a sa propre periode de 200 ms en
/// TEMPS ; cette pause l'allonge au plus de 25 ms.
pub const PAUSE_HID_OISIF_TICKS: u64 = 25;

/// Le sommeil du fil `usb-hid` apres un tour. Un HID servi : un tick, la
/// cadence d'une souris rapide. Rien a lire : la pause d'oisivete -- le fil
/// est `Interactive` et sensible a la latence, epingle au coeur zero ; a
/// la milliseconde il ne servait personne et y preemptait mille fois par
/// seconde (A/B KVM, reveil precis `on`).
pub const fn ticks_apres_tour_hid(hid_servi: bool) -> u64 {
    if hid_servi { 1 } else { PAUSE_HID_OISIF_TICKS }
}
