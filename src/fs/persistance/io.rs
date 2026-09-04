// Disk I/O for an immutable transaction snapshot. Runs at BKL depth 0.

fn synchronise_snapshot(entrees: &[SnapshotEntree]) -> i64 {
    let base = match debut() {
        Some(base) => base,
        None => { oublie_le_disque(); return -1; }
    };
    if entrees.len() > ENTREES_MAX { oublie_le_disque(); return -1; }

    // BOUCHAUD_C5_COMMIT_AB_V1
    //
    // On ecrit dans la demi-zone INACTIVE. Celle dont depend le systeme monte
    // n'est pas touchee : une coupure ici n'abime que du contenu dont personne
    // ne depend, et le superbloc courant continue de designer l'ancien etat.
    let courant = superbloc_courant(base);
    let (emplacement, demi_prevue, generation) = prochain(courant);
    // La demi-zone doit preserver un eventuel etat V1 encore vivant : sur un
    // disque V1, la demi-zone 0 recouvre sa table des sa premiere ecriture,
    // alors que l'en-tete V1 dit toujours « valide ».
    let Some(demi) = demi_sure(demi_prevue) else {
        crate::kernel::dmesg::log(
            "persistance: contenu V1 trop grand pour une demi-zone, migration refusee");
        oublie_le_disque();
        return -1;
    };

    let secteurs_table = secteurs_table_utiles(entrees.len());
    let mut table = vec![0u8; secteurs_table * SECTOR_SIZE];
    let mut secteur = base + contenu_demi(demi);
    let fin_zone = base + fin_demi(demi);
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
            // La demi-zone alterne : un contenu ecrit au tour precedent n'est
            // PAS a la meme place. Le cache d'ecriture ne peut donc sauter une
            // ecriture que si la demi-zone n'a pas change, ce que `deja_ecrite`
            // verifie par le secteur.
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
        let ok = ata::write(Drive::Slave, base + debut_demi(demi), secteurs_table, &table)
            == secteurs_table;
        TX_IO_NS.fetch_add(crate::kernel::timer::monotonic_ns().saturating_sub(io_start), Ordering::Relaxed);
        if !ok { oublie_le_disque(); return -1; }
    }

    // LE COMMIT. Un seul secteur, ecrit dans l'emplacement de superbloc que le
    // montage courant n'utilise PAS. Tant qu'il n'a pas ete ecrit, le systeme
    // monte reste l'ancien -- entierement, jamais a moitie. Et s'il est
    // dechire, sa somme de controle le rejette et l'ancien reste le bon.
    let mut secteur_superbloc = vec![0u8; SECTOR_SIZE];
    let nouveau_superbloc = Superbloc {
        generation,
        demi,
        entrees: entrees.len() as u32,
        secteurs_contenu: secteur.saturating_sub(base + contenu_demi(demi)),
        somme_table: somme_controle(&table),
    };
    if !nouveau_superbloc.encode(&mut secteur_superbloc) {
        oublie_le_disque();
        return -1;
    }
    let io_start = crate::kernel::timer::monotonic_ns();
    let commit_ok = ata::write(
        Drive::Slave, base + emplacement as u64, 1, &secteur_superbloc) == 1;
    TX_IO_NS.fetch_add(crate::kernel::timer::monotonic_ns().saturating_sub(io_start), Ordering::Relaxed);
    if !commit_ok { oublie_le_disque(); return -1; }
    // Le commit a eu lieu : l'etat V1 vient d'etre remplace d'un seul secteur,
    // et il n'y a plus rien a preserver.
    oublie_la_v1();
    TX_COMMITS.fetch_add(1, Ordering::Relaxed);
    TX_GENERATION.store(generation, Ordering::Relaxed);

    *DISQUE.lock() = nouveau;
    TX_WRITTEN.fetch_add(ecrites as u64, Ordering::Relaxed);
    TX_SKIPPED.fetch_add(sautees as u64, Ordering::Relaxed);
    entrees.len() as i64
}
