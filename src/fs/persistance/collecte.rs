/// Tous les fichiers sous `/persist`, chemins relatifs a cette racine.
///
/// Le verrou RAMFS est pris UNE SEULE FOIS pour tout le parcours.  La version
/// precedente gardait `systeme = fs()` ici puis rappelait `fs()` dans
/// `collecte()`. Depuis que RAMFS possede son propre SpinLock non reentrant,
/// le premier sous-repertoire de `/persist` provoquait donc une acquisition
/// recursive sur le meme CPU (observee pendant `fsync`, syscall 74).
fn rassemble() -> Vec<Entree> {
    let systeme = fs();
    rassemble_sous(&systeme)
}

/// Comme [`rassemble`], sous un garde du RAMFS DEJA tenu par l'appelant :
/// celui qui recopie ensuite les contenus doit le faire sous la MEME prise
/// (BOUCHAUD_PERSIST_INSTANTANE_UNIQUE_V1, voir `rassemble_snapshot`).
fn rassemble_sous(systeme: &crate::fs::ramfs::FileSystem) -> Vec<Entree> {
    let racine = match systeme.resolve(RACINE, 0) {
        Some(idx) => idx,
        None => return Vec::new(),
    };

    let mut entrees = Vec::new();
    let mut groupes = 0u32;
    collecte_sous_garde(systeme, racine, &String::new(), None, &mut groupes, &mut entrees);
    entrees
}

/// Le dossier porte-t-il une etiquette `CACHEDIR.TAG` valide ?
///
/// BOUCHAUD_PERSIST_CACHE_JETABLE_V1 : voir `fs/cache_jetable.rs`. Lu sous le
/// garde deja tenu par l'appelant, comme le reste de la collecte.
fn porte_une_etiquette(systeme: &crate::fs::ramfs::FileSystem, dossier: usize) -> bool {
    match systeme.find_child(dossier, crate::fs::cache_jetable::NOM_ETIQUETTE) {
        Some(index) => {
            let noeud = &systeme.nodes[index];
            noeud.kind == NodeKind::File
                && crate::fs::cache_jetable::est_etiquette(
                    noeud.name_str(),
                    &noeud.content[..noeud.content_len()],
                )
        }
        None => false,
    }
}

/// Parcours recursif d'un instantane coherent du RAMFS.
///
/// Cette fonction ne prend JAMAIS le verrou elle-meme : elle emprunte le
/// `FileSystem` deja protege par l'appelant. C'est a la fois plus sur et plus
/// coherent : toute une collecte de persistance voit le meme etat du RAMFS.
fn collecte_sous_garde(
    systeme: &crate::fs::ramfs::FileSystem,
    dossier: usize,
    prefixe: &str,
    groupe: Option<u32>,
    groupes: &mut u32,
    entrees: &mut Vec<Entree>,
) {
    // Relever d'abord les indices permet de ne conserver aucune reference vers
    // un Node au travers de l'appel recursif.
    let mut enfants = Vec::new();
    for index in 0..systeme.nodes.len() {
        if systeme.nodes[index].used
            && systeme.nodes[index].parent == dossier
            && index != dossier
        {
            enfants.push(index);
        }
    }

    for index in enfants {
        let nom = systeme.nodes[index].name_str();
        let chemin = if prefixe.is_empty() {
            String::from(nom)
        } else {
            format!("{}/{}", prefixe, nom)
        };

        match systeme.nodes[index].kind {
            NodeKind::Dir => {
                // Un arbre etiquete devient un groupe jetable ; le plus externe
                // l'emporte, pour qu'un cache dans un cache s'ecarte d'un bloc.
                let groupe = match groupe {
                    Some(g) => Some(g),
                    None if porte_une_etiquette(systeme, index) => {
                        let g = *groupes;
                        *groupes += 1;
                        Some(g)
                    }
                    None => None,
                };
                collecte_sous_garde(systeme, index, &chemin, groupe, groupes, entrees);
            }
            NodeKind::File => {
                let longueur = systeme.nodes[index].content_len();
                if longueur == 0 || chemin.len() >= CHEMIN_MAX {
                    continue;
                }

                // Le contenu reste dans RAMFS. `rassemble_snapshot` le recopie
                // ensuite apres que cette collecte de metadonnees a rendu son
                // garde. Aucun appel a `fs()` n'est imbrique.
                entrees.push(Entree {
                    chemin,
                    noeud: index,
                    longueur,
                    groupe,
                });
            }
        }
    }
}

/// Cree (dossiers compris) puis remplit un fichier sous `/persist`.
///
/// BOUCHAUD_DEPOSE_SOUS_GARDE_V1
///
/// Cette fonction prenait le verrou RAMFS elle-meme. Son unique appelant --
/// `montage::monte` -- le tenait deja, et le noyau mourait au premier fichier
/// restaure :
///
/// ```text
/// *** KERNEL PANIC *** cpu=0
/// panicked at src/fs/ramfs.rs:198:8:
/// LOCKDEP inversion cpu=0 held_rank=50 acquiring=vfs(50)
/// ```
///
/// C'est la MEME faute que celle notee en tete de ce fichier pour
/// `rassemble` : une fonction qui reprend un verrou que son appelant tient
/// deja. Elle avait ete corrigee la et pas ici, parce qu'aucun demarrage de
/// QEMU n'atteignait cette branche -- `hda` faisait zero secteur et
/// `persistance: disque trop petit, zone absente` arretait tout avant. Le
/// premier demarrage avec l'image Ladybird, ou `hda` fait quatre-vingts
/// mebioctets, l'a trouvee tout de suite.
///
/// Elle emprunte donc le `FileSystem` deja protege, comme
/// `collecte_sous_garde`. Le suffixe est celui du module : il DIT que le
/// verrou est tenu ailleurs, et un appelant qui ne le tient pas ne compile
/// pas.
fn depose_sous_garde(
    systeme: &mut crate::fs::ramfs::FileSystem,
    racine: usize,
    chemin: &str,
    contenu: &[u8],
) -> bool {
    let mut parent = racine;
    let mut morceaux = chemin.split('/').filter(|m| !m.is_empty()).peekable();

    while let Some(morceau) = morceaux.next() {
        if morceaux.peek().is_none() {
            let noeud = match systeme.find_child(parent, morceau) {
                Some(idx) => idx,
                None => match systeme.touch_at(parent, morceau) {
                    Ok(idx) => idx,
                    Err(_) => return false,
                },
            };
            return systeme.write_node_bytes(noeud, contenu);
        }

        parent = match systeme.find_child(parent, morceau) {
            Some(idx) => idx,
            None => match systeme.mkdir_at(parent, morceau) {
                Ok(idx) => idx,
                Err(_) => return false,
            },
        };
    }

    false
}
