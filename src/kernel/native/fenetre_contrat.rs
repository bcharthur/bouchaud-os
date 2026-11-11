// BOUCHAUD_BUREAU_RING3_2A -- contrat PUR de `bo_fenetre_lit`.
//
// Inclus par `kernel::native::fenetre` et par le test hote
// `tools/native/test_fenetre_contrat.rs`. Aucun acces au noyau ni au
// materiel : seulement ce qu'une demande de lecture a le droit d'etre.
//
// Le compositeur ring 3 demande au noyau de recopier un rectangle de la
// surface d'une application (la face que le client a livree) dans UNE DE SES
// regions partagees, a un decalage et un pas qu'il choisit -- typiquement sa
// propre image de l'ecran, a la position de la fenetre. Le noyau refuse,
// AVANT toute lecture ou ecriture :
//   - un rectangle vide ;
//   - un rectangle qui sort de ce que le client a reellement peint
//     (`composable`, jamais l'allocation) ;
//   - un pas plus court qu'une ligne du rectangle (lignes qui se chevauchent) ;
//   - une destination qui deborde de la region du compositeur.
// Toutes les sommes et tous les produits sont faits en u64 avec controle :
// aucun debordement arithmetique ne fait passer une demande hors bornes.

/// Taille du fil d'une demande : six `u32` petit-boutistes.
pub const LECTURE_OCTETS: usize = 24;

/// Plus grand cote accepte (le noyau n'alloue jamais de surface plus grande).
pub const COTE_MAX: u32 = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lecture {
    /// Rectangle dans la surface du client.
    pub x: u32,
    pub y: u32,
    pub l: u32,
    pub h: u32,
    /// Octet de la region ou poser le pixel (x, y).
    pub decalage: u32,
    /// Octets entre deux lignes dans la region.
    pub pas: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refus {
    Vide,
    HorsSurface,
    PasTropCourt,
    HorsRegion,
}

impl Lecture {
    pub fn depuis_octets(o: &[u8; LECTURE_OCTETS]) -> Self {
        let m = |i: usize| u32::from_le_bytes([o[4 * i], o[4 * i + 1], o[4 * i + 2], o[4 * i + 3]]);
        Self { x: m(0), y: m(1), l: m(2), h: m(3), decalage: m(4), pas: m(5) }
    }

    /// Octet de la region ou commence la ligne `r` du rectangle. N'est sense
    /// qu'apres `valide`.
    pub fn destination_ligne(&self, r: u32) -> usize {
        (self.decalage as u64 + r as u64 * self.pas as u64) as usize
    }
}

/// `composable` : (largeur, hauteur) que le client a reellement peints.
pub fn valide(d: &Lecture, composable: (u32, u32), octets_region: usize) -> Result<(), Refus> {
    if d.l == 0 || d.h == 0 {
        return Err(Refus::Vide);
    }
    let (cl, ch) = (composable.0.min(COTE_MAX) as u64, composable.1.min(COTE_MAX) as u64);
    if d.x as u64 + d.l as u64 > cl || d.y as u64 + d.h as u64 > ch {
        return Err(Refus::HorsSurface);
    }
    let ligne = d.l as u64 * 4;
    if (d.pas as u64) < ligne {
        return Err(Refus::PasTropCourt);
    }
    // Dernier octet ecrit : decalage + (h - 1) * pas + l * 4.
    let fin = d.decalage as u64 + (d.h as u64 - 1) * d.pas as u64 + ligne;
    if fin > octets_region as u64 {
        return Err(Refus::HorsRegion);
    }
    Ok(())
}
