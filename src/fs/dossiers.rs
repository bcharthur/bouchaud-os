//! Dossiers personnels (BOUCHAUD_DOSSIERS_PERSONNELS_V1).
//!
//! Regles : `dossiers_politique.rs`. Ici, la creation : a l'ouverture de la
//! session, le dossier personnel et ses sous-dossiers standard sont crees
//! s'ils manquent, au nom du compte, en mode prive. L'ecriture differee les
//! rend durables (ils sont sous `/persist`), et la table les retient meme
//! vides (BOUCHAUD_PERSIST_META_V1).

pub use super::dossiers_politique::{chemin, SOUS_DOSSIERS};
use super::dossiers_politique::{MODE_PERSONNEL, MODE_SOUS_DOSSIER, nom_acceptable};
use alloc::string::String;

/// Le dossier personnel de la session courante.
pub fn dossier_personnel() -> String {
    let s = crate::users::session();
    chemin(s.username(), crate::fs::persistance::zone_disponible(), s.home())
}

/// Cree, s'ils manquent, le dossier personnel de la session et ses
/// sous-dossiers. Rend le noeud du dossier personnel. Ne modifie jamais un
/// noeud existant.
pub fn prepare_session() -> Option<usize> {
    let s = crate::users::session();
    let (compte, uid, gid) = (s.username(), s.uid(), s.gid());
    let durable = crate::fs::persistance::zone_disponible();
    let cible = chemin(compte, durable, s.home());
    if !durable || !nom_acceptable(compte) {
        // Sans zone, l'accueil declare reste ce qu'il etait.
        return crate::fs::ramfs::fs().resolve(&cible, 0);
    }
    let mut crees = 0u32;
    let noeud = {
        let mut fs = crate::fs::ramfs::fs();
        let racine = fs.resolve(crate::fs::persistance::RACINE, 0)?;
        let mut cree = |fs: &mut crate::fs::ramfs::FileSystem, parent: usize, nom: &str, mode: u16|
            -> Option<usize>
        {
            match fs.find_child(parent, nom) {
                Some(i) if fs.nodes[i].kind == crate::fs::ramfs::NodeKind::Dir => Some(i),
                Some(_) => None,
                None => {
                    let i = fs.mkdir_at(parent, nom).ok()?;
                    fs.nodes[i].uid = uid;
                    fs.nodes[i].gid = gid;
                    fs.nodes[i].mode = mode;
                    crees += 1;
                    Some(i)
                }
            }
        };
        let home = cree(&mut fs, racine, "home", 0o755)?;
        let perso = cree(&mut fs, home, compte, MODE_PERSONNEL)?;
        for sous in SOUS_DOSSIERS {
            let _ = cree(&mut fs, perso, sous, MODE_SOUS_DOSSIER);
        }
        perso
    };
    crate::serial_println!("BOUCHAUD_DOSSIERS_PERSONNELS chemin={} crees={} durable=1", cible, crees);
    Some(noeud)
}
