/// Deplie la zone persistante dans `/persist`. A appeler une fois au demarrage.
///
/// Rend le nombre de fichiers restaures.
pub fn monte() -> usize {
    // Rien n'est encore connu du disque de ce cote : le premier `sync` ecrira
    // tout. Le contenu lu ici a beau venir du disque, `depose` peut le
    // transformer -- un fichier deja present, un dossier manquant --, et une
    // empreinte tiree d'une lecture ne prouverait donc pas ce que le disque
    // porte apres coup.
    oublie_le_disque();
    let mut systeme = fs();
    let racine = match systeme.resolve(RACINE, 0) {
        Some(idx) => idx,
        None => match systeme.mkdir_at(0, "persist") {
            Ok(idx) => idx,
            Err(_) => return 0,
        },
    };

    let base = match debut() {
        Some(base) => base,
        None => {
            crate::kernel::dmesg::log("persistance: disque trop petit, zone absente");
            return 0;
        }
    };

    // Le format V2 d'abord : deux superblocs, la generation la plus haute
    // parmi les VALIDES. Un superbloc dechire par une coupure pendant le commit
    // est ignore -- ce n'est pas une erreur, c'est precisement le cas que le
    // format existe pour resoudre.
    let (nombre, secteur_table, premier_contenu, somme_attendue) =
        match superbloc_courant(base) {
            Some((_, superbloc)) => {
                // Le disque est deja en V2 : il n'y a plus d'etat V1 a
                // preserver.
                oublie_la_v1();
                let nombre = superbloc.entrees as usize;
                if nombre == 0 || nombre > ENTREES_MAX {
                    crate::kernel::dmesg::log("persistance: superbloc vide");
                    return 0;
                }
                (
                    nombre,
                    base + debut_demi(superbloc.demi),
                    base + contenu_demi(superbloc.demi),
                    Some(superbloc.somme_table),
                )
            }
            None => {
                // Repli V1 : un disque cree avant le chantier 5 reste lisible.
                // La premiere synchronisation le fera passer en V2.
                let mut entete = vec![0u8; SECTOR_SIZE];
                if ata::read(Drive::Slave, base, 1, &mut entete) != 1 {
                    crate::kernel::dmesg::log("persistance: zone illisible");
                    oublie_la_v1();
                    return 0;
                }
                if &entete[0..8] != MAGIE {
                    crate::kernel::dmesg::log("persistance: zone vierge");
                    oublie_la_v1();
                    return 0;
                }
                let nombre = lit_u32(&entete[12..16]) as usize;
                if nombre == 0 || nombre > ENTREES_MAX {
                    return 0;
                }
                TX_MONTAGES_V1.fetch_add(1, Ordering::Relaxed);
                crate::kernel::dmesg::log(
                    "persistance: zone au format V1, migree au prochain sync");
                (nombre, base + 1, base + SECTEUR_CONTENU, None)
            }
        };

    let secteurs_table = secteurs_table_utiles(nombre);
    let mut table = vec![0u8; secteurs_table * SECTOR_SIZE];
    if ata::read(Drive::Slave, secteur_table, secteurs_table, &mut table)
        != secteurs_table
    {
        return 0;
    }

    // La table est verifiee AVANT d'etre suivie. Une table dechiree se
    // manifesterait sinon comme un fichier illisible au hasard, plusieurs
    // minutes plus tard, et sans rapport apparent avec la coupure.
    if let Some(attendue) = somme_attendue {
        if somme_controle(&table) != attendue {
            TX_SUPERBLOCS_REJETES.fetch_add(1, Ordering::Relaxed);
            crate::kernel::dmesg::log(
                "persistance: table incoherente avec son superbloc, zone ignoree");
            return 0;
        }
    }

    let mut restaures = 0usize;
    let mut secteur = premier_contenu;
    for index in 0..nombre {
        let debut_entree = index * TAILLE_ENTREE;
        let brut = &table[debut_entree..debut_entree + TAILLE_ENTREE];
        let taille = lit_u64(&brut[CHEMIN_MAX..CHEMIN_MAX + 8]) as usize;
        let chemin = chaine(&brut[..CHEMIN_MAX]);
        let secteurs = ((taille + SECTOR_SIZE - 1) / SECTOR_SIZE) as u64;

        if !chemin.is_empty() && taille > 0 {
            let mut tampon = vec![0u8; (secteurs as usize) * SECTOR_SIZE];
            if ata::read(Drive::Slave, secteur, secteurs as usize, &mut tampon)
                == secteurs as usize
            {
                tampon.truncate(taille);
                if depose(racine, &chemin, &tampon) {
                    restaures += 1;
                }
            }
        }
        secteur += secteurs;
    }

    // L'ETENDUE de l'etat V1, retenue pour que la premiere ecriture V2 ne la
    // recouvre pas. Le parcours ci-dessus l'a deja calculee : `secteur` pointe
    // juste apres le dernier octet du dernier fichier.
    if somme_attendue.is_none() {
        note_etendue_v1(secteur.saturating_sub(base));
    }

    crate::kernel::dmesg::log_fmt(format_args!(
        "persistance: {} fichier(s) restaure(s) depuis le disque",
        restaures
    ));
    restaures
}
