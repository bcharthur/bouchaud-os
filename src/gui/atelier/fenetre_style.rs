//! Atelier 2 — chrome universel, PUR et sans notion d'application.
//!
//! Ladybird, DOOM, Fichiers, Notes et les autres passent par le meme peintre.
//! Ce contrat n'intervient jamais dans les surfaces clientes : les pixels
//! des applications restent strictement inchanges. Aucune zone de clic,
//! dimension de client, regle de focus ou etendue d'ombre n'est modifiee.
//! Les etats inactifs restent lisibles et les couleurs viennent des jetons.
use super::jetons as j;

/// Toutes les caracteristiques de la barre Atelier commune.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bande {
    pub fond: u32,
    pub titre: u32,
    pub separateur: u32,
    pub liseret: u32,
    pub liseret_alpha: u8,
    pub indicateur: u32,
    pub indicateur_alpha: u8,
}

pub const fn bande(active: bool) -> Bande {
    if active {
        Bande {
            fond: 0xeef0fa,
            titre: j::ENCRE,
            separateur: 0xdde0f0,
            liseret: j::VERRE,
            liseret_alpha: 0xc0,
            indicateur: j::ACCENT,
            indicateur_alpha: 0xd8,
        }
    } else {
        Bande {
            fond: 0xf5f5fa,
            titre: j::ENCRE_DOUCE,
            separateur: j::SEPARATEUR,
            liseret: j::VERRE,
            liseret_alpha: 0x96,
            indicateur: j::ACCENT,
            indicateur_alpha: 0,
        }
    }
}

/// Le liseret indigo reste toujours a l'interieur de la fenetre. Sa
/// largeur ne depend pas de la resolution ni d'une application particuliere.
pub const fn indicateur_largeur(largeur_fenetre: u32) -> u32 {
    let disponible = largeur_fenetre.saturating_sub(32);
    if disponible < 44 { disponible } else { 44 }
}

/// Taille visuelle de chaque capsule, a l'interieur des 40x32 du hit-test.
/// Ne PAS changer les cibles du module windowing.
pub const CAPSULE_LARGEUR: u32 = 22;
pub const CAPSULE_HAUTEUR: u32 = 20;
pub const CAPSULE_RAYON: u32 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bouton {
    pub fond: u32,
    pub fond_alpha: u8,
    pub symbole: u32,
}

/// Aucun bouton n'est peint en rouge tant qu'il n'est pas survole.
/// Les fenetres inactives perdent de la saillance, pas du contraste.
pub const fn bouton(active: bool, survole: bool, fermeture: bool) -> Bouton {
    if survole && fermeture {
        Bouton { fond: j::DANGER, fond_alpha: 255, symbole: j::VERRE }
    } else if survole {
        Bouton { fond: j::ACCENT, fond_alpha: 0x38, symbole: j::ENCRE }
    } else {
        Bouton {
            fond: j::VERRE,
            fond_alpha: if active { 0x95 } else { 0x50 },
            symbole: if active { j::ENCRE_DOUCE } else { j::ENCRE_TENUE },
        }
    }
}
