// V15 facade around the historical widgets implementation.
//
// Le gros `widgets.rs` reste intact. Le chemin event-driven appelle
// `widgets::draw_barre_haute()` ; on reutilise donc son rendu, puis on ajoute un
// indicateur FPS discret avec la MEME police proportionnelle que le bureau.

#[path = "widgets.rs"]
mod legacy;

pub(crate) use legacy::*;


/// Barre superieure.
///
/// # Ce que cette facade ne fait PLUS
///
/// Elle reposait ici un rectangle de fond et le compteur de trames, a une
/// marge droite ecrite en dur. La barre etait donc peinte a deux endroits, et
/// tout element ajoute a droite par `widgets::draw_topbar` finissait dessous :
/// la photo du 12 septembre montre « FPS: 0 necte 17:58:53 », soit la fin de
/// « Deconnecte » recouverte par le compteur.
///
/// Le compteur vit maintenant dans la mise en page unique de la barre, qui
/// avance de la droite vers la gauche et ou chaque element prend la largeur
/// qu'il occupe reellement.
pub(crate) fn draw_barre_haute() {
    legacy::draw_barre_haute();
}
