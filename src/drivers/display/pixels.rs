//! Operations de pixels du tampon arriere, pures (BOUCHAUD_GFX_PIXELS_V1).
//!
//! Aucune dependance au pilote : `tools/gfx/test_pixels.rs` inclut ce fichier
//! tel quel et le compare, pour TOUTES les couvertures et un echantillon dense
//! de couleurs, a la formule d'origine (trois canaux, trois divisions).
//!
//! # Le melange, deux canaux a la fois
//!
//! La formule d'origine, par canal : `(s * a + d * (255 - a)) / 255`. Elle est
//! exacte et le reste ici :
//!
//! * rouge et bleu sont traites ENSEMBLE dans un mot de 32 bits, chacun dans
//!   sa voie de 16 bits. Une voie vaut au plus `255 * 255 = 65 025` : elle ne
//!   deborde jamais sur sa voisine ;
//! * la division par 255 est remplacee par `(x + 1 + (x >> 8)) >> 8`, EXACTE
//!   pour tout `x` de 0 a 65 025 -- donc pour toute valeur qu'une voie peut
//!   prendre (le test hote le verifie sur tout l'intervalle ; elle cesse de
//!   l'etre a 65 535, que la somme d'une voie n'atteint jamais).
//!
//! Deux multiplications par pixel au lieu de six, aucune division, et le
//! resultat est identique au bit pres.
//!
//! # Le remplissage, deux pixels a la fois
//!
//! `fill` sur une tranche de `u32` produit, sans SSE (cible `-sse`), une
//! boucle d'une ecriture de 4 octets par pixel. Sous QEMU TCG, chaque tour de
//! boucle est traduit : c'est le nombre d'iterations qui coute. Ecrire le motif
//! `rgb:rgb` par mots de 64 bits divise ce nombre par deux. Sur x86_64, le
//! pilote passe par `rep stosq` (voir `bochs::remplit`), que le materiel
//! execute par lignes de cache entieres.

/// Melange `rgb` (0x00RRGGBB) sur `dst` a l'opacite `a` (1..=254).
///
/// Identique au bit pres a la formule a trois canaux.
#[inline(always)]
pub fn melange(dst: u32, rgb: u32, a: u32) -> u32 {
    let inv = 255 - a;
    let rb = (dst & 0x00ff_00ff) * inv + (rgb & 0x00ff_00ff) * a;
    let g = ((dst >> 8) & 0xff) * inv + ((rgb >> 8) & 0xff) * a;
    let rb = ((rb + 0x0001_0001 + ((rb >> 8) & 0x00ff_00ff)) >> 8) & 0x00ff_00ff;
    let g = (g + 1 + (g >> 8)) >> 8;
    rb | (g << 8)
}

/// Melange une couleur uniforme sur toute la ligne, a opacite fixe (1..=254).
#[inline]
pub fn melange_ligne(ligne: &mut [u32], rgb: u32, a: u32) {
    // La part de la source ne depend pas du pixel : calculee une fois.
    let inv = 255 - a;
    let s_rb = (rgb & 0x00ff_00ff) * a;
    let s_g = ((rgb >> 8) & 0xff) * a;
    for px in ligne.iter_mut() {
        let d = *px;
        let rb = (d & 0x00ff_00ff) * inv + s_rb;
        let g = ((d >> 8) & 0xff) * inv + s_g;
        let rb = ((rb + 0x0001_0001 + ((rb >> 8) & 0x00ff_00ff)) >> 8) & 0x00ff_00ff;
        let g = (g + 1 + (g >> 8)) >> 8;
        *px = rb | (g << 8);
    }
}

/// Remplissage portable par mots de 64 bits. Reference du test hote, et
/// chemin de toute architecture sans `rep stosq`.
#[inline]
pub fn remplit_portable(ligne: &mut [u32], rgb: u32) {
    let (tete, milieu, queue) = unsafe { ligne.align_to_mut::<u64>() };
    tete.fill(rgb);
    milieu.fill(((rgb as u64) << 32) | rgb as u64);
    queue.fill(rgb);
}
