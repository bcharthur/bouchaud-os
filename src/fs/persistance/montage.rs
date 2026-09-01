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

    let mut entete = vec![0u8; SECTOR_SIZE];
    if ata::read(Drive::Slave, base, 1, &mut entete) != 1 {
        crate::kernel::dmesg::log("persistance: zone illisible");
        return 0;
    }
    if &entete[0..8] != MAGIE {
        // Zone neuve : rien a restaurer, et ce n'est pas une erreur.
        crate::kernel::dmesg::log("persistance: zone vierge");
        return 0;
    }
    let nombre = lit_u32(&entete[12..16]) as usize;
    if nombre == 0 || nombre > ENTREES_MAX {
        return 0;
    }

    // V2 : le nombre d'entrees est dans l'en-tete. Lire les 1024 secteurs
    // reserves quand seules quelques cases sont significatives ne fournit
    // aucune information supplementaire.
    let secteurs_table = secteurs_table_utiles(nombre);
    let mut table = vec![0u8; secteurs_table * SECTOR_SIZE];
    if ata::read(Drive::Slave, base + 1, secteurs_table, &mut table)
        != secteurs_table
    {
        return 0;
    }

    let mut restaures = 0usize;
    let mut secteur = base + SECTEUR_CONTENU;
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

    crate::kernel::dmesg::log_fmt(format_args!(
        "persistance: {} fichier(s) restaure(s) depuis le disque",
        restaures
    ));
    restaures
}
