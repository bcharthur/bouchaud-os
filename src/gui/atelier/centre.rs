//! Centre Atelier : geometrie PURE du centre de controle
//! (BOUCHAUD_ATELIER_CENTRE_V1).
//!
//! # Ce que le centre a le droit de montrer
//!
//! Uniquement ce que le systeme sait faire ou sait dire. Chaque bloc correspond
//! a une information lue au runtime (reseau, affichage) ou a une action qui
//! existe deja ailleurs (redemarrer, eteindre, quitter le bureau -- les memes
//! chemins que le menu Applications). Ce qui n'a pas de pilote -- Wi-Fi, son,
//! theme sombre -- n'a PAS de bouton : une ligne dit qu'il est indisponible.
//! Un interrupteur qui ne commande rien serait un mensonge a l'ecran.
//!
//! # Pourquoi un module sans dessin
//!
//! Le peintre (`widgets::draw_centre`), le test de clic et l'invalidation du
//! survol lisent les MEMES boites. C'est la regle deja appliquee au menu
//! (BOUCHAUD_GUI_HOVER_CONTRAT_V1) et au dock : ce qui est surligne est ce qui
//! se declenche, et le harnais hote le verifie sans framebuffer.

use super::dock::Boite;

/// Largeur du panneau.
pub const LARGEUR: u32 = 320;
/// Marge entre le panneau et le bord droit de l'ecran.
pub const MARGE_DROITE: i32 = 8;
/// Ecart entre la barre du haut et le panneau.
pub const ECART_BARRE: i32 = 6;
/// Rembourrage interieur.
pub const REMBOURRAGE: i32 = 14;
/// Ecart vertical entre deux blocs.
pub const ECART: i32 = 8;
/// Rayon du panneau.
pub const RAYON: u32 = 14;
/// Rayon des blocs interieurs.
pub const RAYON_BLOC: u32 = 10;
/// Debord de l'ombre du panneau, sur les quatre cotes.
pub const OMBRE: i32 = 8;

/// Les blocs, de haut en bas.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bloc {
    Titre,
    Reseau,
    Affichage,
    Diagnostic,
    Indisponibles,
    Alimentation,
}

/// Hauteur de chaque bloc, dans l'ordre d'affichage.
pub const BLOCS: [(Bloc, u32); 6] = [
    (Bloc::Titre, 24),
    (Bloc::Reseau, 58),
    (Bloc::Affichage, 44),
    (Bloc::Diagnostic, 44),
    (Bloc::Indisponibles, 34),
    (Bloc::Alimentation, 62),
];

/// Ce qu'un clic dans le centre peut declencher. Rien d'autre n'est cliquable.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    /// Affiche ou masque le compteur de trames dans la barre du haut.
    Diagnostic,
    Redemarrer,
    Eteindre,
    /// Rend la main au shell (meme chemin que « Quitter » du menu).
    QuitterBureau,
}

pub const ACTIONS: [Action; 4] =
    [Action::Diagnostic, Action::Redemarrer, Action::Eteindre, Action::QuitterBureau];

/// Les trois boutons d'alimentation, de gauche a droite.
pub const ALIMENTATION: [Action; 3] =
    [Action::Redemarrer, Action::Eteindre, Action::QuitterBureau];

/// Hauteur totale du panneau.
pub const fn hauteur() -> u32 {
    let mut h = 2 * REMBOURRAGE as u32;
    let mut i = 0;
    while i < BLOCS.len() {
        h += BLOCS[i].1;
        if i > 0 {
            h += ECART as u32;
        }
        i += 1;
    }
    h
}

/// Cadre du panneau sur un ecran de largeur `l`, sous une barre de `barre` px.
pub fn cadre(l: u32, barre: u32) -> Boite {
    let x = (l as i32 - LARGEUR as i32 - MARGE_DROITE).max(0);
    Boite { x, y: barre as i32 + ECART_BARRE, w: LARGEUR.min(l) as i32, h: hauteur() as i32 }
}

/// Tout ce que le panneau peut peindre, ombre comprise. Bornes du calque et
/// des degats : une seule definition.
pub fn empreinte(l: u32, barre: u32) -> Boite {
    let c = cadre(l, barre);
    let x = (c.x - OMBRE).max(0);
    let y = (c.y - OMBRE).max(0);
    Boite {
        x,
        y,
        w: (c.x + c.w + OMBRE - x).max(0),
        h: (c.y + c.h + OMBRE - y).max(0),
    }
}

/// Boite d'un bloc.
pub fn bloc(l: u32, barre: u32, quel: Bloc) -> Boite {
    let c = cadre(l, barre);
    let mut y = c.y + REMBOURRAGE;
    let x = c.x + REMBOURRAGE;
    let w = (c.w - 2 * REMBOURRAGE).max(0);
    for (b, h) in BLOCS {
        if b == quel {
            return Boite { x, y, w, h: h as i32 };
        }
        y += h as i32 + ECART;
    }
    Boite { x, y, w: 0, h: 0 }
}

/// Boite cliquable d'une action.
pub fn boite_action(l: u32, barre: u32, action: Action) -> Boite {
    match action {
        Action::Diagnostic => bloc(l, barre, Bloc::Diagnostic),
        _ => {
            let a = bloc(l, barre, Bloc::Alimentation);
            let rang = ALIMENTATION.iter().position(|x| *x == action).unwrap_or(0) as i32;
            let n = ALIMENTATION.len() as i32;
            let largeur = ((a.w - (n - 1) * ECART) / n).max(0);
            Boite { x: a.x + rang * (largeur + ECART), y: a.y, w: largeur, h: a.h }
        }
    }
}

/// L'action sous le pointeur, ou `None`. LA definition du survol et du clic.
pub fn action_sous(l: u32, barre: u32, x: i32, y: i32) -> Option<Action> {
    ACTIONS.iter().copied().find(|a| boite_action(l, barre, *a).contient(x, y))
}

/// Le pointeur est-il dans le panneau (ombre exclue) ?
pub fn dans_panneau(l: u32, barre: u32, x: i32, y: i32) -> bool {
    cadre(l, barre).contient(x, y)
}

/// Zone cliquable de la barre du haut qui ouvre le centre : la grappe d'etat
/// de droite, de largeur `grappe` (mesuree par le peintre), plus un
/// rembourrage. Le peintre pose son voile de survol sur CETTE boite.
pub fn bouton_barre(l: u32, barre: u32, grappe: u32) -> Boite {
    let w = grappe as i32 + 2 * BOUTON_REMBOURRAGE;
    let x = (l as i32 - BOUTON_MARGE - w).max(0);
    Boite { x, y: BOUTON_HAUT, w, h: (barre as i32 - 2 * BOUTON_HAUT).max(0) }
}

/// Rembourrage horizontal du bouton de la barre.
pub const BOUTON_REMBOURRAGE: i32 = 8;
/// Marge entre le bouton et le bord droit.
pub const BOUTON_MARGE: i32 = 8;
/// Retrait vertical du bouton dans la barre.
pub const BOUTON_HAUT: i32 = 4;
