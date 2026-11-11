// BOUCHAUD_COMPOSITORD_PRESENTATION_V1 -- contrat PUR de `bo_display_present`.
//
// Inclus par `kernel::native::affichage` et par le test hote
// `tools/native/test_affichage_contrat.rs`. Aucun acces au noyau ni au
// materiel : seulement ce qu'une demande de presentation a le droit d'etre.
//
// Une demande nomme une surface (une region partagee lue comme des pixels
// 32 bits, `largeur x hauteur`, pas = largeur), un rectangle de cette surface
// et sa destination a l'ecran. Le noyau refuse, AVANT toute lecture :
//   - un rectangle vide ou une surface vide ;
//   - une surface plus grande que `PIXELS_MAX` (pas de copie geante) ;
//   - une surface qui deborde de la region partagee (lecture hors region) ;
//   - un rectangle qui sort de la surface ;
//   - une destination qui sort de l'ecran.
// Toutes les sommes et tous les produits sont faits en u64 : aucun
// debordement arithmetique ne peut faire passer une demande hors bornes.

/// Taille du fil d'une demande : huit `u32` petit-boutistes.
pub const DEMANDE_OCTETS: usize = 32;

/// Plus grande surface acceptee (4096 x 4096 pixels).
pub const PIXELS_MAX: u64 = 4096 * 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Demande {
    pub largeur: u32,
    pub hauteur: u32,
    pub x: u32,
    pub y: u32,
    pub l: u32,
    pub h: u32,
    pub dest_x: u32,
    pub dest_y: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refus {
    Vide,
    TropGrand,
    SurfaceHorsRegion,
    HorsSurface,
    HorsEcran,
}

impl Demande {
    pub fn depuis_octets(o: &[u8; DEMANDE_OCTETS]) -> Self {
        let m = |i: usize| u32::from_le_bytes([o[4 * i], o[4 * i + 1], o[4 * i + 2], o[4 * i + 3]]);
        Self { largeur: m(0), hauteur: m(1), x: m(2), y: m(3), l: m(4), h: m(5), dest_x: m(6), dest_y: m(7) }
    }

    /// Decalage, en octets dans la region, du premier pixel de la ligne `r`
    /// du rectangle. N'est sense qu'apres `valide`.
    pub fn decalage_ligne(&self, r: u32) -> usize {
        (((self.y as u64 + r as u64) * self.largeur as u64 + self.x as u64) * 4) as usize
    }
}

pub fn valide(d: &Demande, octets_region: usize, ecran_l: u32, ecran_h: u32) -> Result<(), Refus> {
    if d.largeur == 0 || d.hauteur == 0 || d.l == 0 || d.h == 0 {
        return Err(Refus::Vide);
    }
    let pixels = d.largeur as u64 * d.hauteur as u64;
    if pixels > PIXELS_MAX {
        return Err(Refus::TropGrand);
    }
    if pixels * 4 > octets_region as u64 {
        return Err(Refus::SurfaceHorsRegion);
    }
    if d.x as u64 + d.l as u64 > d.largeur as u64 || d.y as u64 + d.h as u64 > d.hauteur as u64 {
        return Err(Refus::HorsSurface);
    }
    if d.dest_x as u64 + d.l as u64 > ecran_l as u64 || d.dest_y as u64 + d.h as u64 > ecran_h as u64 {
        return Err(Refus::HorsEcran);
    }
    Ok(())
}

/// Empreinte FNV-1a 64 bits de pixels, tronquee a 62 bits pour rester un
/// retour d'appel systeme positif.
pub fn empreinte<I: IntoIterator<Item = u32>>(pixels: I) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in pixels {
        for o in (p & 0x00ff_ffff).to_le_bytes() {
            h ^= o as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h & 0x3fff_ffff_ffff_ffff
}
