// NOTE : commentaires `//` et non `//!` -- ce fichier est inclus tel quel par
// le test hote `tools/fs/test_renommage.rs` (`#[path]`), comme
// `kernel/scheduler/preemption_noyau.rs`.
//
// La DECISION d'un renommage, pure : `rename`, `renameat`, `renameat2`.
//
// BOUCHAUD_RENOMMAGE_POSIX_V1
//
// Le renommage d'avant deplacait le noeud source sous le nom cible sans
// regarder la cible. Si elle existait, le dossier portait ensuite DEUX entrees
// du meme nom ; si la cible etait un dossier non vide, ou un descendant de la
// source, l'arbre se corrompait. `renameat` et `renameat2` n'existaient pas
// (ENOSYS). Ladybird en depend directement : `WebView::FileDownloader` ecrit
// un fichier temporaire puis le RENOMME a sa place, et SQLite et le cache
// HTTP de RequestServer remplacent des fichiers par renommage.
//
// Ce module ne touche a aucun systeme de fichiers : il recoit ce que
// l'appelant a resolu (sous UNE prise du verrou du VFS) et rend soit un code
// d'erreur, soit l'action a appliquer. Les regles sont celles de POSIX
// `rename(2)` et de Linux `renameat2(2)` :
//
//   * drapeaux inconnus, NOREPLACE avec EXCHANGE, WHITEOUT -> EINVAL ;
//   * source absente -> ENOENT ; source racine -> EBUSY ;
//   * dossier parent de la cible absent -> ENOENT, pas un dossier -> ENOTDIR ;
//     nom cible vide, `.` ou `..` -> EINVAL ; trop long -> ENAMETOOLONG ;
//   * NOREPLACE et cible existante -> EEXIST ;
//   * EXCHANGE sans cible -> ENOENT ;
//   * source et cible designent le MEME fichier -> rien a faire, succes ;
//   * un dossier ne peut pas devenir son propre descendant -> EINVAL ;
//   * cible existante : racine -> EBUSY ; dossier <- fichier -> EISDIR ;
//     fichier <- dossier -> ENOTDIR ; dossier non vide -> ENOTEMPTY ;
//   * un noeud adosse au disque (lecture seule) -> EROFS ;
//   * sinon : deplacer (pas de cible), REMPLACER atomiquement (cible
//     existante), ou ECHANGER (EXCHANGE).

pub const RENAME_NOREPLACE: u32 = 1;
pub const RENAME_EXCHANGE: u32 = 2;
pub const RENAME_WHITEOUT: u32 = 4;

pub const ENOENT: i64 = 2;
pub const EBUSY: i64 = 16;
pub const EEXIST: i64 = 17;
pub const ENOTDIR: i64 = 20;
pub const EISDIR: i64 = 21;
pub const EINVAL: i64 = 22;
pub const EROFS: i64 = 30;
pub const ENAMETOOLONG: i64 = 36;
pub const ENOTEMPTY: i64 = 39;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Genre {
    Fichier,
    Dossier,
}

/// Un noeud deja resolu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Noeud {
    pub genre: Genre,
    /// La racine du systeme de fichiers.
    pub racine: bool,
    /// Adosse au disque : lecture seule pour le VFS.
    pub disque: bool,
    /// Pour un dossier : sans enfant.
    pub vide: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nom {
    Valide,
    /// Vide, `.` ou `..`.
    Invalide,
    TropLong,
}

/// Le dossier qui recevra le nom cible.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Parent {
    Absent,
    PasUnDossier,
    Dossier,
}

/// Tout ce que la decision lit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Demande {
    pub drapeaux: u32,
    pub source: Option<Noeud>,
    pub parent_cible: Parent,
    pub nom_cible: Nom,
    pub cible: Option<Noeud>,
    /// La cible existante est le MEME noeud que la source.
    pub meme_noeud: bool,
    /// Le dossier parent de la cible est la source ou l'un de ses descendants.
    pub cible_sous_la_source: bool,
    /// Le dossier parent de la source est la cible ou l'un de ses descendants
    /// (ne compte que pour EXCHANGE, ou les deux noeuds se deplacent).
    pub source_sous_la_cible: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Succes sans rien changer.
    Rien,
    /// Rattacher la source sous le nom cible.
    Deplacer,
    /// Liberer la cible, puis rattacher la source a sa place.
    Remplacer,
    /// Echanger les deux rattachements.
    Echanger,
}

pub fn decide(d: &Demande) -> Result<Action, i64> {
    let connus = RENAME_NOREPLACE | RENAME_EXCHANGE | RENAME_WHITEOUT;
    if d.drapeaux & !connus != 0 {
        return Err(EINVAL);
    }
    if d.drapeaux & RENAME_NOREPLACE != 0 && d.drapeaux & RENAME_EXCHANGE != 0 {
        return Err(EINVAL);
    }
    // Un « whiteout » n'a de sens que pour un systeme de fichiers en couches.
    if d.drapeaux & RENAME_WHITEOUT != 0 {
        return Err(EINVAL);
    }

    let source = match d.source {
        Some(s) => s,
        None => return Err(ENOENT),
    };
    if source.racine {
        return Err(EBUSY);
    }
    match d.parent_cible {
        Parent::Absent => return Err(ENOENT),
        Parent::PasUnDossier => return Err(ENOTDIR),
        Parent::Dossier => {}
    }
    match d.nom_cible {
        Nom::Invalide => return Err(EINVAL),
        Nom::TropLong => return Err(ENAMETOOLONG),
        Nom::Valide => {}
    }
    if d.drapeaux & RENAME_NOREPLACE != 0 && d.cible.is_some() {
        return Err(EEXIST);
    }

    let echange = d.drapeaux & RENAME_EXCHANGE != 0;
    let cible = match d.cible {
        Some(c) => Some(c),
        None if echange => return Err(ENOENT),
        None => None,
    };
    if cible.is_some() && d.meme_noeud {
        return Ok(Action::Rien);
    }
    if source.genre == Genre::Dossier && d.cible_sous_la_source {
        return Err(EINVAL);
    }

    match cible {
        None => {
            if source.disque {
                return Err(EROFS);
            }
            Ok(Action::Deplacer)
        }
        Some(c) if echange => {
            if c.racine {
                return Err(EBUSY);
            }
            if c.genre == Genre::Dossier && d.source_sous_la_cible {
                return Err(EINVAL);
            }
            if source.disque || c.disque {
                return Err(EROFS);
            }
            Ok(Action::Echanger)
        }
        Some(c) => {
            if c.racine {
                return Err(EBUSY);
            }
            match (source.genre, c.genre) {
                (Genre::Fichier, Genre::Dossier) => return Err(EISDIR),
                (Genre::Dossier, Genre::Fichier) => return Err(ENOTDIR),
                (Genre::Dossier, Genre::Dossier) if !c.vide => return Err(ENOTEMPTY),
                _ => {}
            }
            if source.disque || c.disque {
                return Err(EROFS);
            }
            Ok(Action::Remplacer)
        }
    }
}

/// Le nom cible tel que la decision le voit.
pub fn classe_nom(nom: &str, longueur_max: usize) -> Nom {
    if nom.is_empty() || nom == "." || nom == ".." {
        Nom::Invalide
    } else if nom.len() > longueur_max {
        Nom::TropLong
    } else {
        Nom::Valide
    }
}
