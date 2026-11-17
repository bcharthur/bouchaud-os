// BOUCHAUD_SHM_RECT_V1 -- contrat PUR de `bo_shm_fill_rect`.
//
// Inclus par `kernel::native::shm` et par le test hote
// `tools/native/test_shm_rect_contrat.rs`. Aucun acces au noyau.
//
// Le compositeur ring 3 peignait fond, bordures et barres ligne a ligne :
// un `bo_shm_write` (copie depuis l'espace utilisateur + allocation noyau)
// par segment, soit environ quatre appels par ligne d'ecran recomposee. Un
// rectangle plein se decrit en six entiers ; le noyau le remplit sur place,
// sans rien lire de l'appelant.
//
// Le noyau refuse, AVANT toute ecriture :
//   - un rectangle vide ;
//   - une couleur qui ne tient pas sur 32 bits ;
//   - un pas plus court qu'une ligne (lignes qui se chevauchent) ;
//   - un rectangle qui deborde de la region.
// Toutes les sommes et tous les produits sont faits en u64 avec controle :
// aucun debordement arithmetique ne fait passer une demande hors bornes.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Remplissage {
    /// Octet de la region ou commence la premiere ligne.
    pub decalage: usize,
    /// Octets ecrits par ligne (pixels * 4).
    pub ligne: usize,
    pub lignes: usize,
    /// Octets entre deux debuts de ligne.
    pub pas: usize,
    pub couleur: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refus {
    Vide,
    Couleur,
    PasTropCourt,
    HorsRegion,
}

/// `bo_shm_fill_rect(region, decalage, couleur, pixels, lignes, pas)`.
pub fn valide(
    decalage: u64,
    couleur: u64,
    pixels: u64,
    lignes: u64,
    pas: u64,
    octets_region: usize,
) -> Result<Remplissage, Refus> {
    if pixels == 0 || lignes == 0 {
        return Err(Refus::Vide);
    }
    let couleur = u32::try_from(couleur).map_err(|_| Refus::Couleur)?;
    let ligne = pixels.checked_mul(4).ok_or(Refus::HorsRegion)?;
    if pas < ligne {
        return Err(Refus::PasTropCourt);
    }
    // Dernier octet ecrit (exclu) : decalage + (lignes - 1) * pas + ligne.
    let fin = (lignes - 1)
        .checked_mul(pas)
        .and_then(|v| v.checked_add(ligne))
        .and_then(|v| v.checked_add(decalage))
        .ok_or(Refus::HorsRegion)?;
    if fin > octets_region as u64 {
        return Err(Refus::HorsRegion);
    }
    // `fin` tient dans la region, donc dans usize : les conversions sont sures.
    Ok(Remplissage {
        decalage: decalage as usize,
        ligne: ligne as usize,
        lignes: lignes as usize,
        pas: pas as usize,
        couleur,
    })
}

/// Applique un remplissage DEJA valide sur `octets` (la region entiere).
pub fn applique(r: &Remplissage, octets: &mut [u8]) {
    let motif = r.couleur.to_le_bytes();
    for k in 0..r.lignes {
        let debut = r.decalage + k * r.pas;
        for px in octets[debut..debut + r.ligne].chunks_exact_mut(4) {
            px.copy_from_slice(&motif);
        }
    }
}
