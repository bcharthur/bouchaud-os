//! Registre declaratif des applications graphiques EXTERNES.
//!
//! Le noyau ne doit pas savoir comment DOOM ou Ladybird presentent leurs pixels.
//! Atelier n'utilise ici que leur identite, un binaire, des arguments de demarrage
//! et une taille de fenetre. Le protocole GUI, le chargement et le dessin sont
//! identiques pour tous les clients. Premier jalon vers un lanceur ring 3.
use alloc::string::{String, ToString};
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Identite {
    Ladybird,
    Doom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Application {
    pub identite: Identite,
    pub kind: usize,
    pub titre: &'static str,
    pub executable: &'static str,
    pub zone: (i32, i32),
    pub arguments: &'static [&'static str],
}

pub const LADYBIRD: Application = Application {
    identite: Identite::Ladybird,
    kind: 6,
    titre: "Ladybird",
    executable: "/bo-navigateur",
    zone: (1100, 604),
    arguments: &[],
};

pub const DOOM: Application = Application {
    identite: Identite::Doom,
    kind: 11,
    titre: "DOOM - Freedoom",
    executable: "/doom",
    zone: (640, 400),
    arguments: &["-iwad", "/freedoom1.wad", "-nosound"],
};

pub static APPLICATIONS: [Application; 2] = [LADYBIRD, DOOM];

/// Les indices des menus ne sont JAMAIS des indices de ce tableau.
pub fn pour_kind(kind: usize) -> Option<&'static Application> {
    APPLICATIONS.iter().find(|application| application.kind == kind)
}

pub fn pour_chemin(chemin: &str) -> Option<&'static Application> {
    APPLICATIONS.iter().find(|application| application.executable == chemin)
}

/// Nom visible au chargement : jamais un autre programme par defaut.
/// Un programme inconnu reste identificable par le nom de son executable.
pub fn titre_pour_chemin<'a>(chemin: &'a str) -> &'a str {
    match pour_chemin(chemin) {
        Some(application) => application.titre,
        None => {
            let nom = chemin.rsplit('/').next().unwrap_or(chemin);
            if nom.is_empty() { "Application" } else { nom }
        }
    }
}

/// Arguments passés au processus, indépendants des fenêtres et du navigateur.
pub fn argv(chemin: &str) -> Vec<String> {
    let mut arguments = Vec::new();
    arguments.push(chemin.to_string());
    if let Some(application) = pour_chemin(chemin) {
        for argument in application.arguments {
            arguments.push((*argument).to_string());
        }
    }
    arguments
}

/// Le shell appartient à l'OS. L'écran d'attente appartient à Atelier.
/// La navigation, les jeux et leurs barres de progression appartiennent
/// exclusivement aux processus clients une fois leur première trame affichée.
pub const TEXTE_DEMARRAGE: &str = "Ouverture de l'application...";
