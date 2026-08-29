// Disk I/O for an immutable transaction snapshot. Runs at BKL depth 0.

fn synchronise_snapshot(entrees: &[SnapshotEntree]) -> i64 {
    let base = match debut() {
        Some(base) => base,
        None => { oublie_le_disque(); return -1; }
    };
    if entrees.len() > ENTREES_MAX { oublie_le_disque(); return -1; }

    let secteurs_table = secteurs_table_utiles(entrees.len());
    let mut table = vec![0u8; secteurs_table * SECTOR_SIZE];
    let mut secteur = base + SECTEUR_CONTENU;
    let fin_zone = base + SECTEURS_ZONE;
    let mut nouveau: Vec<SurDisque> = Vec::with_capacity(entrees.len());
    let mut ecrites = 0usize;
    let mut sautees = 0usize;

    for (index, entree) in entrees.iter().enumerate() {
        let octets = entree.chemin.as_bytes();
        if octets.len() >= CHEMIN_MAX { oublie_le_disque(); return -1; }
        let longueur = entree.contenu.len();
        let debut_entree = index * TAILLE_ENTREE;
        table[debut_entree..debut_entree + octets.len()].copy_from_slice(octets);
        ecrit_u64(&mut table[debut_entree + CHEMIN_MAX..debut_entree + CHEMIN_MAX + 8], longueur as u64);

        let secteurs = ((longueur + SECTOR_SIZE - 1) / SECTOR_SIZE) as u64;
        if secteur + secteurs > fin_zone { oublie_le_disque(); return -1; }

        let hash_start = crate::kernel::timer::monotonic_ns();
        let sceau_courant = sceau(&entree.contenu);
        TX_HASH_NS.fetch_add(crate::kernel::timer::monotonic_ns().saturating_sub(hash_start), Ordering::Relaxed);

        if secteurs != 0 {
            if deja_ecrite(index, &entree.chemin, secteur, longueur, sceau_courant) {
                sautees += 1;
            } else {
                let mut tampon = vec![0u8; secteurs as usize * SECTOR_SIZE];
                tampon[..longueur].copy_from_slice(&entree.contenu);
                let io_start = crate::kernel::timer::monotonic_ns();
                let ecrits = ata::write(Drive::Slave, secteur, secteurs as usize, &tampon);
                TX_IO_NS.fetch_add(crate::kernel::timer::monotonic_ns().saturating_sub(io_start), Ordering::Relaxed);
                if ecrits != secteurs as usize { oublie_le_disque(); return -1; }
                TX_BYTES.fetch_add(longueur as u64, Ordering::Relaxed);
                ecrites += 1;
            }
        }
        nouveau.push(SurDisque { chemin: entree.chemin.clone(), secteur, longueur, sceau: sceau_courant });
        secteur += secteurs;
    }

    if secteurs_table != 0 {
        let io_start = crate::kernel::timer::monotonic_ns();
        let ok = ata::write(Drive::Slave, base + 1, secteurs_table, &table) == secteurs_table;
        TX_IO_NS.fetch_add(crate::kernel::timer::monotonic_ns().saturating_sub(io_start), Ordering::Relaxed);
        if !ok { oublie_le_disque(); return -1; }
    }

    let mut entete = vec![0u8; SECTOR_SIZE];
    entete[0..8].copy_from_slice(MAGIE);
    ecrit_u32(&mut entete[8..12], 1);
    ecrit_u32(&mut entete[12..16], entrees.len() as u32);
    let io_start = crate::kernel::timer::monotonic_ns();
    let header_ok = ata::write(Drive::Slave, base, 1, &entete) == 1;
    TX_IO_NS.fetch_add(crate::kernel::timer::monotonic_ns().saturating_sub(io_start), Ordering::Relaxed);
    if !header_ok { oublie_le_disque(); return -1; }

    *DISQUE.lock() = nouveau;
    TX_WRITTEN.fetch_add(ecrites as u64, Ordering::Relaxed);
    TX_SKIPPED.fetch_add(sautees as u64, Ordering::Relaxed);
    entrees.len() as i64
}
