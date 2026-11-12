/// BOUCHAUD_MONTAGE_SAIN_V1 -- ce que le montage de CE demarrage a constate.
///
/// Une synchronisation ecrit l'etat COMPLET du RAMFS comme nouvelle
/// generation. Apres un montage qui n'a pas tout restaure, elle effacerait
/// donc du disque ce qui n'a pas ete relu. Trois cas seulement autorisent
/// l'ecriture : pas de zone du tout (rien a perdre), une zone VIERGE (les deux
/// secteurs de superbloc entierement nuls), une restauration COMPLETE. Tout le
/// reste -- zone illisible, superblocs abimes mais non nuls, table illisible
/// ou incoherente, un seul fichier non relu -- bloque toute synchronisation
/// jusqu'au prochain demarrage, et le dit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EtatMontage {
    /// `monte()` n'a pas (encore) ete appele.
    NonMonte,
    /// Pas de zone (disque absent ou trop petit).
    SansZone,
    /// Zone vierge : un volume neuf.
    Neuf,
    /// Restauration complete.
    Restaure,
    /// Echec ou restauration partielle : lecture seule.
    Echec(&'static str),
}

static ETAT_MONTAGE: SpinLock<EtatMontage> = SpinLock::new(EtatMontage::NonMonte);
static REFUS_SIGNALE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

pub fn etat_montage() -> EtatMontage { *ETAT_MONTAGE.lock() }

fn note_montage(e: EtatMontage) {
    *ETAT_MONTAGE.lock() = e;
    REFUS_SIGNALE.store(false, Ordering::Relaxed);
}

/// Vrai quand le dernier montage interdit d'ecrire.
pub fn montage_refuse() -> bool { matches!(etat_montage(), EtatMontage::Echec(_)) }

/// Les deux secteurs de superbloc (et donc l'en-tete V1) sont-ils NULS ?
fn zone_vierge(base: u64) -> Option<bool> {
    let mut deux = vec![0u8; 2 * SECTOR_SIZE];
    if volume_lit(base, 2, &mut deux) != 2 { return None; }
    Some(deux.iter().all(|&b| b == 0))
}

/// Deplie la zone persistante dans `/persist`. A appeler une fois au demarrage.
///
/// Rend le nombre de fichiers restaures ; l'etat du montage se lit avec
/// [`etat_montage`].
pub fn monte() -> usize {
    let (n, e) = monte_avec_etat();
    note_montage(e);
    n
}

fn monte_avec_etat() -> (usize, EtatMontage) {
    // Rien n'est encore connu du disque de ce cote : le premier `sync` ecrira
    // tout. Le contenu lu ici a beau venir du disque, `depose` peut le
    // transformer -- un fichier deja present, un dossier manquant --, et une
    // empreinte tiree d'une lecture ne prouverait donc pas ce que le disque
    // porte apres coup.
    oublie_le_disque();
    // BOUCHAUD_STATE_AB_V1 : le verrou du RAMFS (rang 50, un verrou TOURNANT)
    // n'est JAMAIS tenu pendant une entree-sortie. Sur NVMe -- la partition
    // STATE d'un systeme installe --, une lecture attend en cedant le
    // processeur : la tenir sous ce verrou est une inversion que LOCKDEP
    // transforme en panique (banc run_state_ab.sh, premier montage). Le disque
    // est donc LU d'abord, sans verrou ; le verrou n'est pris qu'ensuite, pour
    // deposer, sans aucune E/S.
    let racine = {
        let mut systeme = fs();
        match systeme.resolve(RACINE, 0) {
            Some(idx) => idx,
            None => match systeme.mkdir_at(0, "persist") {
                Ok(idx) => idx,
                Err(_) => return (0, EtatMontage::Echec("racine_persist")),
            },
        }
    };

    let base = match debut() {
        Some(base) => base,
        None => {
            crate::kernel::dmesg::log("persistance: disque trop petit, zone absente");
            return (0, EtatMontage::SansZone);
        }
    };
    // BOUCHAUD_ECRITURE_DIFFEREE_V1 : la zone existe -- vierge ou non --, et
    // toute modification sous cette racine est desormais comptee. Le montage
    // lui-meme est exclu : sa generation devient la reference du fil
    // (`note_fin_montage`), y compris quand il s'arrete sur une zone vierge.
    crate::fs::ramfs::note_racine_persistante(racine);
    crate::fs::ecriture_differee::note_fin_montage();

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
                if nombre > ENTREES_MAX {
                    crate::kernel::dmesg::log("persistance: superbloc invalide");
                    return (0, EtatMontage::Echec("superbloc_invalide"));
                }
                if nombre == 0 {
                    // Un etat committe VIDE : fidele, rien a restaurer.
                    crate::kernel::dmesg::log("persistance: superbloc vide");
                    return (0, EtatMontage::Restaure);
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
                if volume_lit(base, 1, &mut entete) != 1 {
                    crate::kernel::dmesg::log("persistance: zone illisible");
                    oublie_la_v1();
                    return (0, EtatMontage::Echec("zone_illisible"));
                }
                if &entete[0..8] != MAGIE {
                    oublie_la_v1();
                    // VIERGE seulement si les deux superblocs sont NULS. Des
                    // superblocs abimes mais non nuls ne sont pas un disque
                    // neuf : c'est un etat qu'on ne sait pas relire, et
                    // l'ecraser le detruirait.
                    return match zone_vierge(base) {
                        Some(true) => {
                            crate::kernel::dmesg::log("persistance: zone vierge");
                            (0, EtatMontage::Neuf)
                        }
                        Some(false) => {
                            crate::kernel::dmesg::log("persistance: superblocs illisibles, zone non vierge");
                            (0, EtatMontage::Echec("superblocs_illisibles"))
                        }
                        None => (0, EtatMontage::Echec("zone_illisible")),
                    };
                }
                let nombre = lit_u32(&entete[12..16]) as usize;
                if nombre > ENTREES_MAX {
                    return (0, EtatMontage::Echec("v1_invalide"));
                }
                if nombre == 0 {
                    return (0, EtatMontage::Restaure);
                }
                TX_MONTAGES_V1.fetch_add(1, Ordering::Relaxed);
                crate::kernel::dmesg::log(
                    "persistance: zone au format V1, migree au prochain sync");
                (nombre, base + 1, base + SECTEUR_CONTENU, None)
            }
        };

    let secteurs_table = secteurs_table_utiles(nombre);
    let mut table = vec![0u8; secteurs_table * SECTOR_SIZE];
    if volume_lit(secteur_table, secteurs_table, &mut table)
        != secteurs_table
    {
        return (0, EtatMontage::Echec("table_illisible"));
    }

    // La table est verifiee AVANT d'etre suivie. Une table dechiree se
    // manifesterait sinon comme un fichier illisible au hasard, plusieurs
    // minutes plus tard, et sans rapport apparent avec la coupure.
    if let Some(attendue) = somme_attendue {
        if somme_controle(&table) != attendue {
            TX_SUPERBLOCS_REJETES.fetch_add(1, Ordering::Relaxed);
            crate::kernel::dmesg::log(
                "persistance: table incoherente avec son superbloc, zone ignoree");
            return (0, EtatMontage::Echec("table_incoherente"));
        }
    }

    // 1. LIRE, sans verrou : (chemin, meta, contenu ; None = dossier ou
    //    lecture echouee, comme avant).
    let mut lus: Vec<(String, Option<Meta>, Option<Vec<u8>>, bool)> = Vec::new();
    let mut non_relus = 0usize;
    let mut secteur = premier_contenu;
    for index in 0..nombre {
        let debut_entree = index * TAILLE_ENTREE;
        let brut = &table[debut_entree..debut_entree + TAILLE_ENTREE];
        let taille = lit_u64(&brut[CHEMIN_MAX..CHEMIN_MAX + 8]) as usize;
        let chemin = chaine(&brut[..CHEMIN_MAX]);
        let secteurs = ((taille + SECTOR_SIZE - 1) / SECTOR_SIZE) as u64;
        // BOUCHAUD_PERSIST_META_V1 : genre, droits et proprietaire. Absents
        // (disque d'avant), l'entree est un fichier aux droits par defaut.
        let meta = Meta::decode(&brut[DEBUT_META..TAILLE_ENTREE]);

        if a_restaurer(&chemin, taille, meta) {
            if meta.map_or(false, |m| m.dossier) {
                lus.push((chemin, meta, None, true));
            } else {
                let mut tampon = vec![0u8; (secteurs as usize) * SECTOR_SIZE];
                if volume_lit(secteur, secteurs as usize, &mut tampon) == secteurs as usize {
                    tampon.truncate(taille);
                    lus.push((chemin, meta, Some(tampon), false));
                } else {
                    non_relus += 1;
                }
            }
        }
        secteur += secteurs;
    }

    // 2. DEPOSER, sous verrou, sans E/S.
    let mut restaures = 0usize;
    {
        let mut systeme = fs();
        for (chemin, meta, contenu, dossier) in &lus {
            let noeud = if *dossier {
                dossier_sous_garde(&mut systeme, racine, chemin)
            } else if let Some(tampon) = contenu {
                depose_noeud_sous_garde(&mut systeme, racine, chemin, tampon)
            } else {
                None
            };
            if let Some(idx) = noeud {
                if let Some(m) = meta {
                    systeme.nodes[idx].mode = m.mode;
                    systeme.nodes[idx].uid = m.uid;
                    systeme.nodes[idx].gid = m.gid;
                }
                restaures += 1;
            }
        }
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
    crate::fs::ecriture_differee::note_fin_montage();
    if non_relus > 0 {
        crate::kernel::dmesg::log_fmt(format_args!(
            "persistance: {} fichier(s) NON relu(s) : lecture seule pour ce demarrage", non_relus));
        return (restaures, EtatMontage::Echec("restauration_partielle"));
    }
    (restaures, EtatMontage::Restaure)
}
