//! Fichiers Atelier : geometrie, cibles et modele PURS
//! (BOUCHAUD_ATELIER_FICHIERS_V1).
//!
//! # Ce que ce module decide
//!
//! Ou se trouve chaque chose dans la fenetre Fichiers -- barre de navigation,
//! fil d'Ariane, barre laterale, grille ou liste, barre d'etat -- et ce que
//! designe un point. Le peintre (`apps::file_explorer`), le clic, le survol et
//! le defilement lisent les MEMES boites : ce qui est surligne est ce qui
//! s'ouvre (la regle BOUCHAUD_GUI_HOVER_CONTRAT_V1 du menu, appliquee ici).
//!
//! Il decide aussi comment on CLASSE et comment on NOMME un element (type par
//! extension, taille lisible). Il ne lit pas le RAMFS : le peintre lui donne
//! des `Entree`, le harnais hote aussi.
//!
//! # Ce qu'il ne fait pas
//!
//! Rien qui touche au stockage ou aux droits. Les raccourcis de la barre
//! laterale designent des chemins ; s'ils n'existent pas, l'application les
//! montre absents -- elle ne les cree pas.

use alloc::string::String;
use alloc::vec::Vec;
use super::dock::Boite;

/// Hauteur de la barre de navigation.
pub const BARRE_H: i32 = 44;
/// Hauteur de la barre d'etat.
pub const ETAT_H: i32 = 26;
/// Largeur de la barre laterale (quand la fenetre est assez large).
pub const LATERALE_L: i32 = 176;
/// En dessous de cette largeur, la barre laterale se replie.
pub const LARGEUR_MIN_LATERALE: i32 = 520;
/// Cote des boutons de la barre.
pub const BOUTON: i32 = 28;
/// Cellule de la grille.
pub const CELLULE_L: i32 = 104;
pub const CELLULE_H: i32 = 100;
/// Marge interieure de la zone de contenu.
pub const MARGE: i32 = 12;
/// Icone de la grille.
pub const ICONE_GRILLE: i32 = 48;
/// Ligne de la liste, et son en-tete.
pub const LIGNE_H: i32 = 30;
pub const ENTETE_LISTE_H: i32 = 28;
/// Ligne de la barre laterale, et titre de section.
pub const RACCOURCI_H: i32 = 30;
pub const SECTION_H: i32 = 28;
/// Separateur du fil d'Ariane, et marque de troncature.
pub const SEPARATEUR_L: i32 = 18;
pub const ELLIPSE_L: i32 = 22;

/// Mode d'affichage du contenu.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Vue {
    Grille,
    Liste,
}

/// Un raccourci de la barre laterale : libelle, chemin (`~` = accueil),
/// section (0 Favoris, 1 Emplacements).
pub const RACCOURCIS: [(&str, &str, u8); 7] = [
    ("Accueil", "~", 0),
    ("Documents", "~/Documents", 0),
    ("Images", "~/Images", 0),
    ("T\u{e9}l\u{e9}chargements", "~/T\u{e9}l\u{e9}chargements", 0),
    ("Racine", "/", 1),
    ("Applications", "/apps", 1),
    ("Temporaire", "/tmp", 1),
];

/// Titres des sections de la barre laterale.
pub const SECTIONS: [&str; 2] = ["Favoris", "Emplacements"];

/// Ce que designe un point de la fenetre.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cible {
    Retour,
    Avance,
    Parent,
    /// Segment `i` du fil d'Ariane (0 = racine).
    Segment(usize),
    VueGrille,
    VueListe,
    Raccourci(usize),
    /// Element `i` de la liste triee du dossier.
    Element(usize),
    /// Contenu, hors de tout element (desselectionne).
    Fond,
    Rien,
}

/// Geometrie d'une fenetre Fichiers dont la zone utile mesure `l x h`.
/// Toutes les boites sont relatives au coin haut gauche de cette zone.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Geometrie {
    pub l: i32,
    pub h: i32,
}

impl Geometrie {
    pub const fn new(l: i32, h: i32) -> Self {
        Self { l, h }
    }

    pub const fn largeur_laterale(&self) -> i32 {
        if self.l >= LARGEUR_MIN_LATERALE { LATERALE_L } else { 0 }
    }

    pub const fn barre(&self) -> Boite {
        Boite { x: 0, y: 0, w: self.l, h: BARRE_H }
    }

    pub const fn laterale(&self) -> Boite {
        let h = self.h - BARRE_H - ETAT_H;
        Boite { x: 0, y: BARRE_H, w: self.largeur_laterale(), h: if h > 0 { h } else { 0 } }
    }

    pub const fn contenu(&self) -> Boite {
        let x = self.largeur_laterale();
        let h = self.h - BARRE_H - ETAT_H;
        Boite { x, y: BARRE_H, w: self.l - x, h: if h > 0 { h } else { 0 } }
    }

    pub const fn etat(&self) -> Boite {
        Boite { x: 0, y: self.h - ETAT_H, w: self.l, h: ETAT_H }
    }

    /// Boite d'un bouton de la barre.
    pub const fn bouton(&self, c: Cible) -> Boite {
        let y = (BARRE_H - BOUTON) / 2;
        let x = match c {
            Cible::Retour => 10,
            Cible::Avance => 10 + BOUTON + 4,
            Cible::Parent => 10 + 2 * (BOUTON + 4),
            Cible::VueGrille => self.l - 10 - 2 * BOUTON - 4,
            Cible::VueListe => self.l - 10 - BOUTON,
            _ => return Boite { x: 0, y: 0, w: 0, h: 0 },
        };
        Boite { x, y, w: BOUTON, h: BOUTON }
    }

    /// Zone du fil d'Ariane, entre les boutons de navigation et ceux de vue.
    pub const fn zone_ariane(&self) -> Boite {
        let x = 10 + 3 * (BOUTON + 4) + 10;
        let fin = self.l - 10 - 2 * BOUTON - 4 - 12;
        Boite { x, y: (BARRE_H - BOUTON) / 2, w: if fin > x { fin - x } else { 0 }, h: BOUTON }
    }

    /// Ligne du raccourci `i` dans la barre laterale.
    pub fn raccourci(&self, i: usize) -> Boite {
        let lat = self.laterale();
        if lat.w == 0 || i >= RACCOURCIS.len() {
            return Boite { x: 0, y: 0, w: 0, h: 0 };
        }
        let section = RACCOURCIS[i].2 as i32;
        let avant = RACCOURCIS[..i].len() as i32;
        let y = lat.y + 8 + (section + 1) * SECTION_H + section * 10 + avant * RACCOURCI_H;
        Boite { x: lat.x + 8, y, w: lat.w - 16, h: RACCOURCI_H }
    }

    /// Ordonnee du titre de la section `s`.
    pub fn titre_section(&self, s: usize) -> i32 {
        let premier = RACCOURCIS.iter().position(|r| r.2 as usize == s).unwrap_or(0);
        self.raccourci(premier).y - SECTION_H
    }

    /// Colonnes de la grille.
    pub const fn colonnes(&self) -> usize {
        let c = self.contenu();
        let utile = c.w - 2 * MARGE;
        let n = utile / CELLULE_L;
        if n < 1 { 1 } else { n as usize }
    }

    /// Hauteur d'une rangee, selon la vue.
    pub const fn pas(vue: Vue) -> i32 {
        match vue {
            Vue::Grille => CELLULE_H,
            Vue::Liste => LIGNE_H,
        }
    }

    /// Haut de la zone des elements (sous l'en-tete de liste).
    pub const fn haut_elements(&self, vue: Vue) -> i32 {
        let c = self.contenu();
        match vue {
            Vue::Grille => c.y + MARGE,
            Vue::Liste => c.y + ENTETE_LISTE_H,
        }
    }

    /// Rangees entierement visibles.
    pub const fn rangees_visibles(&self, vue: Vue) -> usize {
        let c = self.contenu();
        let h = c.y + c.h - self.haut_elements(vue);
        let n = h / Self::pas(vue);
        if n < 1 { 1 } else { n as usize }
    }

    /// Rangees au total pour `n` elements.
    pub const fn rangees(&self, n: usize, vue: Vue) -> usize {
        match vue {
            Vue::Grille => (n + self.colonnes() - 1) / self.colonnes(),
            Vue::Liste => n,
        }
    }

    /// Defilement maximal (en rangees) : la derniere rangee reste en bas.
    pub const fn defilement_max(&self, n: usize, vue: Vue) -> usize {
        let r = self.rangees(n, vue);
        let v = self.rangees_visibles(vue);
        if r > v { r - v } else { 0 }
    }

    /// Boite de l'element `i` avec `defil` rangees defilees, ou vide s'il est
    /// hors de la zone visible.
    pub fn element(&self, i: usize, vue: Vue, defil: usize) -> Boite {
        let c = self.contenu();
        let (rang, x, w) = match vue {
            Vue::Grille => {
                let cols = self.colonnes();
                let rang = i / cols;
                let col = (i % cols) as i32;
                (rang, c.x + MARGE + col * CELLULE_L, CELLULE_L)
            }
            Vue::Liste => (i, c.x + 6, c.w - 12),
        };
        if rang < defil {
            return Boite { x: 0, y: 0, w: 0, h: 0 };
        }
        let visible = rang - defil;
        if visible >= self.rangees_visibles(vue) {
            return Boite { x: 0, y: 0, w: 0, h: 0 };
        }
        let y = self.haut_elements(vue) + visible as i32 * Self::pas(vue);
        Boite { x, y, w, h: Self::pas(vue) }
    }

    /// L'element sous `(x, y)` parmi `n`, ou `None`.
    pub fn element_sous(&self, x: i32, y: i32, n: usize, vue: Vue, defil: usize) -> Option<usize> {
        let c = self.contenu();
        if !c.contient(x, y) || y < self.haut_elements(vue) {
            return None;
        }
        let rang = ((y - self.haut_elements(vue)) / Self::pas(vue)) as usize;
        if rang >= self.rangees_visibles(vue) {
            return None;
        }
        let i = match vue {
            Vue::Grille => {
                let dx = x - c.x - MARGE;
                if dx < 0 {
                    return None;
                }
                let col = (dx / CELLULE_L) as usize;
                if col >= self.colonnes() {
                    return None;
                }
                (rang + defil) * self.colonnes() + col
            }
            Vue::Liste => rang + defil,
        };
        (i < n && self.element(i, vue, defil).contient(x, y)).then_some(i)
    }

    /// Ce que designe `(x, y)`. `ariane` : boites des segments visibles du fil
    /// d'Ariane, avec l'indice de leur segment (voir [`segments`]).
    pub fn cible(&self, x: i32, y: i32, n: usize, vue: Vue, defil: usize,
                 ariane: &[(usize, Boite)]) -> Cible {
        for b in [Cible::Retour, Cible::Avance, Cible::Parent, Cible::VueGrille, Cible::VueListe] {
            if self.bouton(b).contient(x, y) {
                return b;
            }
        }
        if let Some(&(i, _)) = ariane.iter().find(|(_, b)| b.contient(x, y)) {
            return Cible::Segment(i);
        }
        if let Some(i) = (0..RACCOURCIS.len()).find(|&i| self.raccourci(i).contient(x, y)) {
            return Cible::Raccourci(i);
        }
        if let Some(i) = self.element_sous(x, y, n, vue, defil) {
            return Cible::Element(i);
        }
        if self.contenu().contient(x, y) {
            return Cible::Fond;
        }
        Cible::Rien
    }
}

/// Place les segments du fil d'Ariane dans `zone`, de gauche a droite, a
/// partir de leurs LARGEURS de texte mesurees (+ rembourrage). Quand tout ne
/// tient pas, on garde la racine et les derniers segments, et une ellipse
/// prend la place du milieu. Rend `(indice du segment, boite)` pour chaque
/// segment visible, et la boite de l'ellipse s'il y en a une.
pub fn segments(zone: Boite, largeurs: &[i32]) -> (Vec<(usize, Boite)>, Option<Boite>) {
    let n = largeurs.len();
    let total = |idx: &[usize], ellipse: bool| -> i32 {
        idx.iter().map(|&i| largeurs[i]).sum::<i32>()
            + (idx.len() as i32 - 1).max(0) * SEPARATEUR_L
            + if ellipse { ELLIPSE_L + SEPARATEUR_L } else { 0 }
    };
    let tous: Vec<usize> = (0..n).collect();
    let (garde, ellipse) = if n == 0 || total(&tous, false) <= zone.w {
        (tous, false)
    } else {
        // La racine, puis le plus de derniers segments possible.
        let mut garde = Vec::new();
        for debut in 1..n {
            let mut essai = Vec::from([0usize]);
            essai.extend(debut..n);
            if total(&essai, true) <= zone.w {
                garde = essai;
                break;
            }
        }
        if garde.is_empty() {
            // Meme le dernier seul ne tient pas avec la racine : le dernier.
            garde = Vec::from([n - 1]);
        }
        (garde, true)
    };
    let mut x = zone.x;
    let mut boites = Vec::new();
    let mut boite_ellipse = None;
    for (k, &i) in garde.iter().enumerate() {
        if k > 0 {
            x += SEPARATEUR_L;
        }
        if ellipse && k == 1 && garde[0] == 0 {
            boite_ellipse = Some(Boite { x, y: zone.y, w: ELLIPSE_L, h: zone.h });
            x += ELLIPSE_L + SEPARATEUR_L;
        }
        let w = largeurs[i].min(zone.x + zone.w - x).max(0);
        boites.push((i, Boite { x, y: zone.y, w, h: zone.h }));
        x += largeurs[i];
    }
    (boites, boite_ellipse)
}

// ─── Modele ────────────────────────────────────────────────────────────────

/// Le genre d'un element, pour son icone et sa colonne « Type ».
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Genre {
    Dossier,
    Fichier,
    Texte,
    Image,
    Code,
    Archive,
    Executable,
}

/// Le genre d'apres le nom (extension) ; un dossier est un dossier.
pub fn genre(nom: &str, dossier: bool) -> Genre {
    if dossier {
        return Genre::Dossier;
    }
    let ext = match nom.rfind('.') {
        Some(p) if p > 0 && p + 1 < nom.len() => &nom[p + 1..],
        _ => return Genre::Fichier,
    };
    let e: String = ext.chars().map(|c| c.to_ascii_lowercase()).collect();
    match e.as_str() {
        "txt" | "md" | "log" | "conf" | "cfg" | "ini" | "csv" => Genre::Texte,
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "ppm" | "svg" | "webp" => Genre::Image,
        "rs" | "c" | "h" | "cpp" | "py" | "sh" | "js" | "ts" | "html" | "css" | "json"
        | "toml" | "yaml" | "yml" => Genre::Code,
        "zip" | "tar" | "gz" | "tgz" | "xz" | "bz2" | "7z" => Genre::Archive,
        "elf" | "bin" | "exe" | "so" => Genre::Executable,
        _ => Genre::Fichier,
    }
}

/// Libelle de la colonne « Type ».
pub const fn libelle(g: Genre) -> &'static str {
    match g {
        Genre::Dossier => "Dossier",
        Genre::Fichier => "Fichier",
        Genre::Texte => "Texte",
        Genre::Image => "Image",
        Genre::Code => "Code source",
        Genre::Archive => "Archive",
        Genre::Executable => "Ex\u{e9}cutable",
    }
}

/// Un element d'un dossier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entree {
    pub nom: String,
    pub dossier: bool,
    pub octets: usize,
    /// Indice du noeud RAMFS (opaque ici).
    pub noeud: usize,
}

/// Ordre d'affichage : les dossiers d'abord, puis par nom sans tenir compte
/// de la casse, puis par nom exact (ordre total, donc stable d'une trame a
/// l'autre : la selection designe toujours le meme element).
pub fn trie(entrees: &mut [Entree]) {
    entrees.sort_by(|a, b| {
        b.dossier.cmp(&a.dossier)
            .then_with(|| {
                let la = a.nom.chars().map(|c| c.to_lowercase().next().unwrap_or(c));
                let lb = b.nom.chars().map(|c| c.to_lowercase().next().unwrap_or(c));
                la.cmp(lb)
            })
            .then_with(|| a.nom.cmp(&b.nom))
            .then_with(|| a.noeud.cmp(&b.noeud))
    });
}

/// Taille lisible, en unites francaises (o, Ko, Mo).
pub fn taille_lisible(octets: usize) -> String {
    use alloc::format;
    if octets < 1024 {
        format!("{} o", octets)
    } else if octets < 1024 * 1024 {
        format!("{} Ko", (octets + 512) / 1024)
    } else {
        let dixiemes = (octets * 10 + 512 * 1024) / (1024 * 1024);
        format!("{},{} Mo", dixiemes / 10, dixiemes % 10)
    }
}

/// Deplace une selection au clavier : `delta` rangees (haut/bas) ou elements
/// (gauche/droite) selon la vue, borne a `[0, n)`.
pub fn deplace_selection(sel: Option<usize>, n: usize, delta: i32, cols: usize) -> Option<usize> {
    if n == 0 {
        return None;
    }
    let base = match sel {
        Some(s) => s.min(n - 1) as i64,
        None => return Some(0),
    };
    let pas = delta as i64 * cols.max(1) as i64;
    Some((base + pas).clamp(0, n as i64 - 1) as usize)
}

/// Le defilement qui garde `sel` visible.
pub fn defilement_pour(sel: usize, defil: usize, g: &Geometrie, vue: Vue) -> usize {
    let rang = match vue {
        Vue::Grille => sel / g.colonnes(),
        Vue::Liste => sel,
    };
    let v = g.rangees_visibles(vue);
    if rang < defil {
        rang
    } else if rang >= defil + v {
        rang + 1 - v
    } else {
        defil
    }
}

// ─── Menu contextuel ───────────────────────────────────────────────────────

/// Largeur du menu contextuel, hauteur d'une entree, rembourrage.
pub const MENU_L: i32 = 212;
pub const MENU_ENTREE_H: i32 = 30;
pub const MENU_REMBOURRAGE: i32 = 6;

/// Une action du menu contextuel. Chacune appelle un chemin qui existe deja
/// (naviguer, presse-papiers, changer de vue) : aucune entree decorative.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Ouvrir,
    /// Ouvre un fichier texte dans Notes (BOUCHAUD_NOTES_OUVRIR_V1).
    OuvrirNotes,
    CopierChemin,
    Proprietes,
    VueGrille,
    VueListe,
}

/// Les entrees du menu, selon ce qui est sous le pointeur.
/// Les entrees du menu, selon ce qui est sous le pointeur. `texte` : le
/// fichier vise s'edite dans Notes ([`editable_dans_notes`]).
pub fn actions(sur_element: bool, dossier: bool, texte: bool) -> &'static [Action] {
    match (sur_element, dossier, texte) {
        (true, true, _) => &[Action::Ouvrir, Action::CopierChemin, Action::Proprietes],
        (true, false, true) => &[Action::OuvrirNotes, Action::CopierChemin, Action::Proprietes],
        (true, false, false) => &[Action::CopierChemin, Action::Proprietes],
        (false, _, _) => &[Action::VueGrille, Action::VueListe, Action::CopierChemin],
    }
}

/// Taille maximale d'un fichier que Notes accepte d'ouvrir (sa limite).
pub const NOTES_MAX_OCTETS: usize = 256 * 1024;

/// Un fichier s'edite-t-il dans Notes ? Texte, code, ou sans extension, et
/// dans la limite de Notes. Notes verifie aussi l'UTF-8 a l'ouverture.
pub fn editable_dans_notes(g: Genre, octets: usize) -> bool {
    matches!(g, Genre::Texte | Genre::Code | Genre::Fichier) && octets <= NOTES_MAX_OCTETS
}

/// Libelle d'une action. `sur_element` distingue « Copier le chemin » de
/// « Copier le chemin du dossier ».
pub const fn libelle_action(a: Action, sur_element: bool) -> &'static str {
    match a {
        Action::Ouvrir => "Ouvrir",
        Action::OuvrirNotes => "Ouvrir dans Notes",
        Action::CopierChemin => if sur_element { "Copier le chemin" } else { "Copier le chemin du dossier" },
        Action::Proprietes => "Propri\u{e9}t\u{e9}s",
        Action::VueGrille => "Afficher en grille",
        Action::VueListe => "Afficher en liste",
    }
}

impl Geometrie {
    /// Boite du menu de `n` entrees ouvert en `(x, y)`, gardee dans la zone
    /// utile (un menu qui deborde de la fenetre serait peint hors d'elle).
    pub fn menu(&self, x: i32, y: i32, n: usize) -> Boite {
        let h = n as i32 * MENU_ENTREE_H + 2 * MENU_REMBOURRAGE;
        let w = MENU_L.min(self.l);
        let x = x.min(self.l - w).max(0);
        let y = y.min(self.h - h).max(0);
        Boite { x, y, w, h }
    }

    /// Entree `i` d'un menu de boite `m`.
    pub const fn entree_menu(m: Boite, i: usize) -> Boite {
        Boite {
            x: m.x + MENU_REMBOURRAGE,
            y: m.y + MENU_REMBOURRAGE + i as i32 * MENU_ENTREE_H,
            w: m.w - 2 * MENU_REMBOURRAGE,
            h: MENU_ENTREE_H,
        }
    }

    /// L'entree sous `(x, y)` dans un menu de `n` entrees, ou `None`.
    pub fn entree_sous(m: Boite, n: usize, x: i32, y: i32) -> Option<usize> {
        (0..n).find(|&i| Self::entree_menu(m, i).contient(x, y))
    }
}

// ─── Historique ────────────────────────────────────────────────────────────

/// Historique de navigation d'une fenetre (Product M1), borne a 32 entrees
/// dans chaque sens. Les noeuds anciens sont REVALIDES avant chaque retour :
/// un dossier supprime entre-temps est saute, jamais ouvert.
pub struct Historique {
    previous: Vec<usize>,
    following: Vec<usize>,
}
impl Historique {
    pub fn new() -> Self { Self { previous: Vec::new(), following: Vec::new() } }
    pub fn visit(&mut self, current: usize, destination: usize) {
        if current == destination { return; }
        if self.previous.len() >= 32 { self.previous.remove(0); }
        self.previous.push(current);
        self.following.clear();
    }
    pub fn back(&mut self, current: usize, valid: impl Fn(usize) -> bool) -> Option<usize> {
        while let Some(node) = self.previous.pop() {
            if valid(node) {
                if self.following.len() >= 32 { self.following.remove(0); }
                self.following.push(current);
                return Some(node);
            }
        }
        None
    }
    pub fn forward(&mut self, current: usize, valid: impl Fn(usize) -> bool) -> Option<usize> {
        while let Some(node) = self.following.pop() {
            if valid(node) {
                if self.previous.len() >= 32 { self.previous.remove(0); }
                self.previous.push(current);
                return Some(node);
            }
        }
        None
    }
    pub fn peut_reculer(&self) -> bool { !self.previous.is_empty() }
    pub fn peut_avancer(&self) -> bool { !self.following.is_empty() }
}

