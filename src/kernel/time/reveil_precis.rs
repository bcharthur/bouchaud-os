//! Le reveil a l'echeance exacte d'un coeur au repos (BOUCHAUD_REVEIL_PRECIS_V1, P13+).
//!
//! # Pourquoi
//!
//! Une tache endormie jusqu'a une echeance n'etait reveillee que lorsqu'un
//! coeur balayait les echeances : au tick du PIT (1 ms, coeur 0) ou au quantum
//! local (4 ms). Au repos, le retard de reveil valait donc jusqu'a un tick.
//! Run 38645396730 (KVM, sonde d'ordonnanceur) : retard au repos median
//! 1,3 ms, p95 1,9 ms, PLUS que sous charge -- la sonde rendait « charge sans
//! effet mesurable » a toutes les topologies. L'arrondi au millieme des
//! sommeils est corrige (BOUCHAUD_SOMMEIL_NS_V1) ; reste la detection.
//!
//! # Le modele
//!
//! Juste avant `hlt` (interruptions coupees), un coeur en mode TSC-deadline
//! arme son minuteur local sur la PLUS PROCHE de deux echeances : celle de son
//! quantum, ou la borne inferieure des echeances de sommeil
//! (`Echeances::borne`). Au reveil, l'ordonnanceur balaie et la tache due
//! devient prete. Si le coeur est reveille par autre chose avant, l'armement
//! precis est retire (le quantum est rearme) : il ne peut jamais tomber sur une
//! tache en cours et lui imputer un quantum.
//!
//! Ce module ne decide que l'echeance a armer ; pur, teste sur l'hote
//! (`tools/smp/test_reveil_precis.rs`).

/// « Aucune echeance en vue » (`echeances::JAMAIS`).
pub const JAMAIS: u64 = u64::MAX;
/// En deca, armer couterait plus qu'attendre : le coeur ne s'endort pas
/// vraiment, et une echeance deja due sera vue au prochain balayage.
pub const MARGE_NS: u64 = 20_000;

/// Le TSC qui correspond a l'instant monotone `ns`, avec l'horloge
/// `monotonic_ns = (tsc - boot) * 1e9 / hz`. `None` si `hz` est inconnu ou si
/// le resultat deborde.
pub fn tsc_de(ns: u64, boot_tsc: u64, hz: u64) -> Option<u64> {
    if hz == 0 {
        return None;
    }
    let cycles = (ns as u128).checked_mul(hz as u128)? / 1_000_000_000u128;
    let tsc = (boot_tsc as u128).checked_add(cycles)?;
    u64::try_from(tsc).ok()
}

/// L'echeance TSC a armer avant `hlt`, ou `None` pour laisser le quantum.
///
/// `borne_ns` : la plus proche echeance de sommeil connue (borne inferieure ;
/// `JAMAIS` pendant un balayage) ; `maintenant_ns` / `tsc_maintenant` :
/// l'heure ; `arme_tsc` : ce que le minuteur local porte deja (le quantum) ;
/// `cible_tsc` : `tsc_de(borne_ns)` ; `voulu` : la politique.
pub fn decide(
    voulu: bool,
    borne_ns: u64,
    maintenant_ns: u64,
    tsc_maintenant: u64,
    arme_tsc: u64,
    cible_tsc: Option<u64>,
) -> Option<u64> {
    if !voulu || borne_ns == JAMAIS || borne_ns == 0 {
        return None;
    }
    if borne_ns <= maintenant_ns.saturating_add(MARGE_NS) {
        return None;
    }
    let cible = cible_tsc?;
    if cible <= tsc_maintenant || cible >= arme_tsc {
        return None;
    }
    Some(cible)
}

/// La meme decision pour un minuteur local en mode PERIODIQUE (QEMU TCG n'a
/// pas TSC-deadline) : le compte a ecrire pour tirer a l'echeance, si elle
/// precede ce qui reste du compte courant. `comptes_par_ms` : la frequence du
/// minuteur apres son diviseur. Le quantum est restaure au declenchement.
pub fn decide_compte(
    voulu: bool,
    borne_ns: u64,
    maintenant_ns: u64,
    compte_restant: u32,
    comptes_par_ms: u64,
) -> Option<u32> {
    if !voulu || borne_ns == JAMAIS || borne_ns == 0 || comptes_par_ms == 0 {
        return None;
    }
    if borne_ns <= maintenant_ns.saturating_add(MARGE_NS) {
        return None;
    }
    let delai = borne_ns - maintenant_ns;
    let compte = (delai as u128 * comptes_par_ms as u128).div_ceil(1_000_000).max(1);
    if compte >= compte_restant as u128 {
        return None;
    }
    Some(compte as u32)
}
