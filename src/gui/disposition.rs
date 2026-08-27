//! Geometrie pure du bureau : QUEL element occupe QUEL rectangle.
//!
//! # Pourquoi ce module existe
//!
//! Le compositeur repond deux fois a la meme question, dans deux fichiers
//! differents :
//!
//!   * `window_manager::plan_de_scene` dit OU un calque a le droit de peindre ;
//!   * les appels a `Degats::ajoute` disent QUOI invalider quand l'etat change.
//!
//! Tant que ces deux reponses sont ecrites separement, elles peuvent diverger
//! sans que rien ne le signale : le code compile, les compteurs montent, et
//! l'ecran ment. C'est exactement ce qui s'est produit avec l'horloge — le
//! tic invalidait la barre des TACHES alors que `Element::BarreHaute` peint
//! l'heure en haut. `frames_clock_only` augmentait chaque seconde, et
//! `HH:MM:SS` restait fige.
//!
//! Ce module est donc la definition unique. Le plan de scene et les degats en
//! derivent tous les deux ; ils ne peuvent plus se contredire.
//!
//! # Pourquoi il est pur
//!
//! Il ne connait ni le framebuffer, ni l'allocateur, ni le temps. Il prend les
//! dimensions en parametre. Cela le rend testable sur l'hote — c'est la
//! condition pour que l'oracle de transition d'etat puisse verifier, pixel par
//! pixel, que le degat annonce par une mutation suffit a rendre l'image.

// Chemin RELATIF, comme `scene.rs` : c'est ce qui permet aux tests de l'hote
// d'inclure ce fichier tel quel via `#[path]`, sans le noyau autour.
use super::protocole::Rect;

/// Hauteur des barres haut et bas. Doit rester egale a `window::BAR_H`.
pub const HAUTEUR_BARRE: u32 = 11;

/// Debord de l'ombre portee des fenetres et du menu, en pixels.
///
/// Doit rester egal a `widgets::DEBORD_OMBRE`.
pub const DEBORD_OMBRE: u32 = 4;

/// Hauteur d'une entree du menu Demarrer. Egale a `window::MENU_ITEM_H`.
pub const HAUTEUR_LIGNE_MENU: i32 = 22;

/// Bandeau vide en haut du menu. Egale a `window::MENU_HEADER_H`.
pub const ENTETE_MENU: i32 = 8;

/// Largeur de la bande d'accent bleue, a gauche du menu.
///
/// `draw_menu` la peint puis commence les lignes juste apres : c'est aussi la
/// borne gauche de la zone sensible au survol.
pub const BANDE_ACCENT: i32 = 4;

/// Empreinte du curseur logiciel (fleche 12x19), volontairement un peu large.
pub const LARGEUR_CURSEUR: u32 = 14;
pub const HAUTEUR_CURSEUR: u32 = 22;

/// Barre du HAUT : titre, statistiques CPU/RAM/Disque, horloge.
///
/// C'est ici que se trouvent les seuls pixels du bureau qui changent tout
/// seuls. Voir `widgets::draw_topbar`.
pub const fn barre_haute(largeur: u32) -> Rect {
    Rect::neuf(0, 0, largeur, HAUTEUR_BARRE)
}

/// Barre du BAS : bouton Demarrer et boutons de fenetres.
///
/// Rien n'y change avec le temps : elle ne depend que de la liste des fenetres
/// et de l'ouverture du menu.
pub const fn barre_taches(largeur: u32, hauteur: u32) -> Rect {
    Rect::neuf(
        0,
        hauteur.saturating_sub(HAUTEUR_BARRE) as i32,
        largeur,
        HAUTEUR_BARRE,
    )
}

/// Empreinte du curseur a une position donnee.
pub const fn curseur(x: i32, y: i32) -> Rect {
    Rect::neuf(x, y, LARGEUR_CURSEUR, HAUTEUR_CURSEUR)
}

/// Ce qu'un cadre OCCUPE reellement : lui-meme plus son ombre portee.
///
/// L'ombre est decalee vers le bas et la droite ; un degat limite au cadre
/// laisse donc une bande sombre derriere tout ce qui bouge.
pub fn empreinte_avec_ombre(cadre: Rect) -> Rect {
    if cadre.vide() {
        return cadre;
    }
    Rect::neuf(
        cadre.x,
        cadre.y,
        cadre.largeur.saturating_add(DEBORD_OMBRE),
        cadre.hauteur.saturating_add(DEBORD_OMBRE),
    )
}

/// Nombre de lignes du menu, deduit de sa hauteur.
///
/// `menu_rect().h = n * HAUTEUR_LIGNE_MENU + ENTETE_MENU + 8`.
pub fn lignes_menu(menu: Rect) -> usize {
    let utile = menu.hauteur as i32 - ENTETE_MENU - 8;
    if utile <= 0 {
        return 0;
    }
    (utile / HAUTEUR_LIGNE_MENU).max(0) as usize
}

/// Ligne du menu sous le pointeur, ou `None`.
///
/// C'est LA definition du survol. `widgets::draw_menu` l'appelle pour savoir
/// quelle ligne mettre en valeur, et le gestionnaire de fenetres l'appelle pour
/// savoir quelles lignes invalider. Une seule definition, donc aucun ecart
/// possible entre ce qui est peint et ce qui est presente.
pub fn ligne_menu_survolee(menu: Rect, mx: i32, my: i32) -> Option<usize> {
    let lignes = lignes_menu(menu);
    if lignes == 0 {
        return None;
    }
    if mx < menu.x + BANDE_ACCENT || (mx as i64) >= menu.droite() {
        return None;
    }
    let relatif = my - menu.y - ENTETE_MENU;
    if relatif < 0 || relatif >= lignes as i32 * HAUTEUR_LIGNE_MENU {
        return None;
    }
    Some((relatif / HAUTEUR_LIGNE_MENU) as usize)
}

/// Rectangle de la ligne `index` du menu.
///
/// MAJORE volontairement la zone que le survol repeint : il part du bord gauche
/// du menu, bande d'accent comprise, alors que le fond survole commence apres.
/// Un degat trop grand coute des pixels ; un degat trop petit laisse un reste
/// de surbrillance a l'ecran.
pub fn rect_ligne_menu(menu: Rect, index: usize) -> Rect {
    if index >= lignes_menu(menu) {
        return Rect::default();
    }
    Rect::neuf(
        menu.x,
        menu.y + ENTETE_MENU + index as i32 * HAUTEUR_LIGNE_MENU,
        menu.largeur,
        HAUTEUR_LIGNE_MENU as u32,
    )
}
