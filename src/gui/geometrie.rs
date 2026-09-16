//! Rectangle arrondi : un test d'appartenance qui ne peut pas paniquer.
//!
//! BOUCHAUD_ARRONDI_TOTAL_V1
//!
//! # La panique, relevee le 16 septembre 2026
//!
//! ```text
//! *** KERNEL PANIC *** cpu=0
//! panicked at src/gui/apps/file_explorer.rs:163:17:
//! min > max. min = 447, max = 446
//! [FAULT] task=3 pid=7 tid=103 nom=desktop
//! ```
//!
//! Ouvrir le gestionnaire de fichiers tuait le noyau. La cause tient en une
//! ligne de constantes :
//!
//! ```text
//! dans_arrondi(nx, ny, 285, haut, 735, haut + 34, 17)
//! ```
//!
//! Une boite de trente-quatre de haut, un rayon de dix-sept. Les centres d'arc
//! etaient calcules ainsi :
//!
//! ```text
//! cy = py.clamp(y0 + rayon, y1 - rayon - 1)
//!    = py.clamp(haut + 17, haut + 16)
//! ```
//!
//! `min > max`, et `clamp` panique. Pour `ligne = 0`, `haut` vaut 430 : d'ou
//! 447 et 446, les deux nombres du releve.
//!
//! Le test n'etait atteint que pour un point DEDANS la boite -- d'ou une
//! machine qui demarre, affiche un bureau, et meurt au premier fichier
//! dessine.
//!
//! # Ce que le rayon veut dire, et ce qu'il ne peut pas dire
//!
//! Un rayon de coin ne peut pas depasser la moitie de la boite : au-dela, les
//! quatre arcs se chevauchent et la forme demandee n'existe pas. L'ancienne
//! version faisait confiance a l'appelant ; celle-ci BORNE le rayon, ce qui
//! rend la fonction totale -- aucune entree ne peut la faire paniquer, et un
//! rayon trop grand donne la forme la plus proche qui ait un sens : un stade.
//!
//! # Pourquoi ce fichier est partage
//!
//! La meme primitive existait en deux exemplaires : ici pour les icones du
//! bureau, et dans `reference_gop` pour le logo de demarrage. La seconde est
//! dessinee AVANT que quoi que ce soit sache rapporter une faute -- une
//! panique y serait une machine muette. Deux copies d'un meme calcul, c'est
//! deux fois la meme faute a trouver.

/// Le rayon de coin effectivement utilisable pour une boite donnee.
///
/// Rend zero pour une boite degeneree : un rectangle sans surface n'a pas de
/// coins a arrondir, et c'est le seul cas ou l'on ne peut rien dessiner.
pub fn rayon_utilisable(largeur: i32, hauteur: i32, rayon: i32) -> i32 {
    if largeur <= 0 || hauteur <= 0 {
        return 0;
    }
    // `(cote - 1) / 2` et non `cote / 2` : les bornes de centre d'arc vont de
    // `x0 + r` a `x1 - r - 1` INCLUSES, et il faut que la premiere reste sous
    // la seconde. Avec une largeur de 34 et un rayon de 17, `34 / 2` rendrait
    // encore 17 et la panique reviendrait.
    rayon.max(0).min((largeur - 1) / 2).min((hauteur - 1) / 2)
}

/// Le point `(px, py)` est-il dans le rectangle `[x0, x1) x [y0, y1)` a coins
/// arrondis ?
///
/// Totale : aucune combinaison d'entrees ne panique, y compris une boite vide,
/// inversee, ou un rayon plus grand qu'elle.
pub fn dans_arrondi(
    px: i32,
    py: i32,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    rayon: i32,
) -> bool {
    if px < x0 || py < y0 || px >= x1 || py >= y1 {
        return false;
    }
    // Pas de cas particulier pour `r == 0` : le point etant deja dans la
    // boite, `clamp(x0, x1 - 1)` le rend inchange, et la distance vaut zero.
    // Une branche de plus serait du code qu'aucun test ne peut distinguer.
    let r = rayon_utilisable(x1 - x0, y1 - y0, rayon);
    let cx = px.clamp(x0 + r, x1 - r - 1);
    let cy = py.clamp(y0 + r, y1 - r - 1);
    let dx = (px - cx) as i64;
    let dy = (py - cy) as i64;
    dx * dx + dy * dy <= (r as i64) * (r as i64)
}
