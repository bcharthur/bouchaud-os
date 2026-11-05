//! Première brique VFS : canonicalisation stricte des chemins absolus.
//! Ce module ne monte PAS encore de volumes et ne remplace PAS le RAMFS.
//! Il empêche toutefois un document ouvert par Fichiers de viser un chemin
//! ambigu, trop long ou sortant au-dessus de la racine.
use alloc::string::String;
use alloc::vec::Vec;

pub const MAX_CHEMIN: usize = 4096;
pub const MAX_COMPOSANT: usize = 255;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErreurChemin {
    Relatif,
    TropLong,
    ComposantTropLong,
    ControleInterdit,
    SortieRacine,
}

pub fn normalise_absolu(chemin: &str) -> Result<String, ErreurChemin> {
    if !chemin.starts_with('/') {
        return Err(ErreurChemin::Relatif);
    }
    if chemin.len() > MAX_CHEMIN {
        return Err(ErreurChemin::TropLong);
    }
    if chemin.chars().any(|c| c.is_control()) {
        return Err(ErreurChemin::ControleInterdit);
    }
    let mut composants: Vec<&str> = Vec::new();
    for p in chemin.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                if composants.pop().is_none() {
                    return Err(ErreurChemin::SortieRacine);
                }
            }
            _ => {
                if p.len() > MAX_COMPOSANT {
                    return Err(ErreurChemin::ComposantTropLong);
                }
                composants.push(p);
            }
        }
    }
    let mut resultat = String::from("/");
    for (i, p) in composants.iter().enumerate() {
        if i != 0 { resultat.push('/'); }
        resultat.push_str(p);
    }
    Ok(resultat)
}
