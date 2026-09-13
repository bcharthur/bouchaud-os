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
