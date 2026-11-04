//! Geometrie du dock flottant (BOUCHAUD_ATELIER_DOCK_V1).
//!
//! Pur : prend les dimensions de l'ecran et le nombre de fenetres, rend des
//! boites. Le peintre (`widgets::draw_taskbar`), le test d'impact
//! (`window::start_btn`, `window::taskbar_btn`) et les degats
//! (`disposition::barre_taches`) en derivent tous : ils ne peuvent pas
//! designer deux docks differents.
//!
//! # La forme
//!
//! Une pastille de verre centree en bas de l'ecran, a `MARGE_BAS` du bord.
//! A gauche, la cellule Applications (le menu) ; puis un separateur ; puis une
//! cellule par fenetre ouverte, dans l'ordre de la pile. Le dock grandit et
//! retrecit avec le nombre de fenetres, toujours centre.
//!
//! # La zone
//!
//! La BANDE du bas (`HAUTEUR_ZONE`) contient le dock, sa marge et son ombre.
//! C'est elle que le compositeur invalide quand le dock change : sa largeur
//! etant celle de l'ecran, un dock qui retrecit ne laisse rien derriere lui.

/// Cote d'une cellule (icone + respiration).
pub const CELLULE: i32 = 52;
/// Cote de l'icone dans une cellule. Egal a `icones::TAILLE_DOCK`.
pub const ICONE: i32 = 40;
/// Ecart entre deux cellules.
pub const ECART: i32 = 4;
/// Rembourrage interieur du dock.
pub const REMBOURRAGE: i32 = 8;
/// Largeur reservee au separateur (filet + ses deux marges).
pub const SEPARATEUR: i32 = 17;
/// Hauteur du dock lui-meme.
pub const HAUTEUR: i32 = CELLULE + 2 * REMBOURRAGE;
/// Ecart entre le dock et le bas de l'ecran.
pub const MARGE_BAS: i32 = 8;
/// Debord de l'ombre du dock (egal a `disposition::DEBORD_OMBRE`).
pub const OMBRE: i32 = 8;
/// Bande du bas reservee au dock : dock + marge + ombre au-dessus.
pub const HAUTEUR_ZONE: u32 = (HAUTEUR + MARGE_BAS + OMBRE) as u32;
/// Marge laterale minimale entre le dock et les bords de l'ecran.
pub const MARGE_LATERALE: i32 = 24;

/// Une boite en coordonnees ecran.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Boite {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Boite {
    pub const fn contient(&self, px: i32, py: i32) -> bool {
        self.w > 0 && self.h > 0
            && px >= self.x && px < self.x + self.w
            && py >= self.y && py < self.y + self.h
    }
}

/// Nombre de cellules de fenetre que l'ecran peut montrer.
pub const fn capacite(largeur: u32) -> usize {
    let utile = largeur as i32 - 2 * MARGE_LATERALE - 2 * REMBOURRAGE - CELLULE - SEPARATEUR;
    if utile < CELLULE {
        return 0;
    }
    ((utile + ECART) / (CELLULE + ECART)) as usize
}

/// Fenetres reellement montrees, bornees par la capacite.
pub const fn visibles(largeur: u32, fenetres: usize) -> usize {
    let c = capacite(largeur);
    if fenetres < c { fenetres } else { c }
}

/// Le dock entier.
pub const fn cadre(largeur: u32, hauteur: u32, fenetres: usize) -> Boite {
    let n = visibles(largeur, fenetres) as i32;
    let mut w = 2 * REMBOURRAGE + CELLULE;
    if n > 0 {
        w += SEPARATEUR + n * CELLULE + (n - 1) * ECART;
    }
    Boite {
        x: (largeur as i32 - w) / 2,
        y: hauteur as i32 - MARGE_BAS - HAUTEUR,
        w,
        h: HAUTEUR,
    }
}

/// La cellule Applications (ouvre le menu).
pub const fn cellule_applications(largeur: u32, hauteur: u32, fenetres: usize) -> Boite {
    let d = cadre(largeur, hauteur, fenetres);
    Boite { x: d.x + REMBOURRAGE, y: d.y + REMBOURRAGE, w: CELLULE, h: CELLULE }
}

/// Abscisse du filet separateur.
pub const fn x_separateur(largeur: u32, hauteur: u32, fenetres: usize) -> i32 {
    let d = cadre(largeur, hauteur, fenetres);
    d.x + REMBOURRAGE + CELLULE + SEPARATEUR / 2
}

/// La cellule de la fenetre `index`. Boite vide si elle n'a pas de place.
pub const fn cellule_fenetre(largeur: u32, hauteur: u32, fenetres: usize, index: usize) -> Boite {
    if index >= visibles(largeur, fenetres) {
        return Boite { x: 0, y: 0, w: 0, h: 0 };
    }
    let d = cadre(largeur, hauteur, fenetres);
    let x0 = d.x + REMBOURRAGE + CELLULE + SEPARATEUR;
    Boite {
        x: x0 + index as i32 * (CELLULE + ECART),
        y: d.y + REMBOURRAGE,
        w: CELLULE,
        h: CELLULE,
    }
}

// ─── Survol (BOUCHAUD_ATELIER_DOCK_SURVOL_V1) ──────────────────────────────
//
// Ce que le pointeur designe dans le dock. LA definition, pour le peintre
// (voile, levee de l'icone), l'infobulle, l'invalidation et le clic : elle ne
// lit que `cellule_applications` et `cellule_fenetre`, les memes boites que
// `window::start_btn` et `window::taskbar_btn`.

/// Une cellule du dock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cible {
    Applications,
    Fenetre(usize),
}

/// Levee de l'icone survolee, en pixels, au terme du fondu. Elle reste DANS
/// la cellule : la zone de clic ne bouge pas avec l'image.
pub const LEVEE: i32 = 3;
/// Hauteur de l'infobulle.
pub const INFOBULLE_H: i32 = 24;
/// Ecart entre l'infobulle et le haut du dock.
pub const INFOBULLE_ECART: i32 = 8;
/// Rembourrage horizontal du texte de l'infobulle.
pub const INFOBULLE_REMBOURRAGE: i32 = 10;

/// La cellule sous `(x, y)`, ou `None`.
pub fn cible_sous(largeur: u32, hauteur: u32, fenetres: usize, x: i32, y: i32) -> Option<Cible> {
    if cellule_applications(largeur, hauteur, fenetres).contient(x, y) {
        return Some(Cible::Applications);
    }
    (0..visibles(largeur, fenetres))
        .find(|&i| cellule_fenetre(largeur, hauteur, fenetres, i).contient(x, y))
        .map(Cible::Fenetre)
}

/// La boite d'une cible.
pub fn boite_cible(largeur: u32, hauteur: u32, fenetres: usize, cible: Cible) -> Boite {
    match cible {
        Cible::Applications => cellule_applications(largeur, hauteur, fenetres),
        Cible::Fenetre(i) => cellule_fenetre(largeur, hauteur, fenetres, i),
    }
}

/// L'infobulle d'une cible dont le libelle mesure `texte` pixels : centree
/// au-dessus de la cellule, gardee dans l'ecran.
pub fn infobulle(largeur: u32, hauteur: u32, fenetres: usize, cible: Cible, texte: i32) -> Boite {
    let c = boite_cible(largeur, hauteur, fenetres, cible);
    let d = cadre(largeur, hauteur, fenetres);
    let w = texte.max(0) + 2 * INFOBULLE_REMBOURRAGE;
    let x = (c.x + c.w / 2 - w / 2).clamp(4, (largeur as i32 - 4 - w).max(4));
    Boite { x, y: d.y - INFOBULLE_ECART - INFOBULLE_H, w, h: INFOBULLE_H }
}
