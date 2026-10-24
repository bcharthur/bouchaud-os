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
/// Delai donne a une echeance DEJA DUE au moment d'armer (BOUCHAUD_REVEIL_DU_V1).
///
/// Ce n'est plus un seuil de refus. Jusqu'ici une echeance due, ou a moins de
/// cette marge, n'etait pas armee : « le prochain balayage la servira ». Mais
/// le coeur faisait quand meme `hlt`, et le prochain balayage etait la
/// prochaine interruption -- le tic du PIT (1 ms, coeur zero) ou le quantum
/// (4 ms). Run 39308093631 (KVM, reveil precis actif) : retard au repos p50
/// 35-51 us, p95 851-955 us -- un reveil sur vingt attendait le tic. La sonde
/// rendait « charge sans effet » (repos p95 917 us > charge p95 212 us) : le
/// coeur AU REPOS reveillait plus tard que le coeur charge.
///
/// Desormais une echeance dans la marge est armee EXACTEMENT, et une echeance
/// due l'est a `maintenant + MARGE_NS` : jamais plus d'une marge de retard,
/// et une borne perimee (due mais sans tache a servir) ne peut faire tirer le
/// minuteur qu'une fois par marge -- pas en boucle.
pub const MARGE_NS: u64 = 20_000;

/// An IPI or sleep wakeup before the saved boundary is not a quantum tick.
/// Applies to both LAPIC backends, including an idle sleep's deadline.
pub fn avant_quantum(maintenant: u64, quantum: u64) -> bool {
    quantum != 0 && maintenant < quantum
}

/// Restore only the remaining interval, never grant a fresh quantum after
/// a device IRQ interrupts HLT. An overdue boundary fires at the next count.
pub fn reste_compte(quantum_ns: u64, maintenant_ns: u64, comptes_par_ms: u64) -> u32 {
    ((quantum_ns.saturating_sub(maintenant_ns) as u128 * comptes_par_ms as u128)
        .div_ceil(1_000_000)).clamp(1, u32::MAX as u128) as u32
}

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

/// La borne a armer sur CE coeur (BOUCHAUD_REVEIL_ECHEANCE_PROPRE_V1) : la
/// plus proche de la borne globale des sommeils et de l'echeance de la tache
/// bloquee installee sur ce coeur (`propre`, 0 : aucune).
///
/// La borne globale vaut `JAMAIS` pendant le balayage d'un AUTRE coeur. Un
/// coeur qui s'endort a ce moment n'armait rien -- meme pour la tache qui
/// dort chez lui : son reveil passait alors par le balayage d'un autre coeur
/// puis par un IPI, soit le reveil d'un processeur virtuel arrete (run
/// 39387288347, KVM SMP4 : `refus_sans_borne` ~10 % des armements, reprises
/// au repos de 113 a 382 us). L'echeance propre ne depend d'aucun balayage.
pub fn borne_du_coeur(globale: u64, propre: u64) -> u64 {
    let globale = if globale == 0 { JAMAIS } else { globale };
    let propre = if propre == 0 { JAMAIS } else { propre };
    globale.min(propre)
}

/// L'instant a armer sur un coeur OCCUPE pour la borne des sommeils
/// interactifs (BOUCHAUD_REVEIL_PRECIS_OCCUPE_V1) : la borne elle-meme, ou
/// `maintenant + MARGE_NS` si elle est deja due (une borne perimee ne fait pas
/// tirer le minuteur en boucle). `None` sans borne.
pub fn echeance_occupee(borne_ns: u64, maintenant_ns: u64) -> Option<u64> {
    if borne_ns == JAMAIS || borne_ns == 0 {
        return None;
    }
    Some(if borne_ns <= maintenant_ns { maintenant_ns.saturating_add(MARGE_NS) } else { borne_ns })
}

/// Pourquoi une echeance n'est PAS armee.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refus {
    /// Politique desactivee.
    Politique,
    /// Aucune echeance connue (ou balayage en cours sur un autre coeur : il
    /// recale la borne et l'armera lui-meme en s'endormant).
    SansBorne,
    /// Frequence du TSC inconnue.
    CibleInconnue,
    /// Meme `maintenant + MARGE_NS` est deja passe sur ce TSC : horloges
    /// incoherentes, rien de sur a armer.
    DejaPasse,
    /// Le quantum arme tombe avant l'echeance : il reste arme.
    QuantumAvant,
}

/// Ce qui est arme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Armement {
    /// Le TSC a ecrire dans `IA32_TSC_DEADLINE`.
    pub tsc: u64,
    /// L'echeance etait deja due (ou sa cible deja passee sur ce TSC) :
    /// armee a `maintenant + MARGE_NS`.
    pub due: bool,
}

/// `decide`, avec le motif du refus.
///
/// `cible_tsc` : `tsc_de(borne_ns)` ; `plancher_tsc` :
/// `tsc_de(maintenant_ns + MARGE_NS)`.
pub fn decide_motif(
    voulu: bool,
    borne_ns: u64,
    maintenant_ns: u64,
    tsc_maintenant: u64,
    arme_tsc: u64,
    cible_tsc: Option<u64>,
    plancher_tsc: Option<u64>,
) -> Result<Armement, Refus> {
    if !voulu {
        return Err(Refus::Politique);
    }
    if borne_ns == JAMAIS || borne_ns == 0 {
        return Err(Refus::SansBorne);
    }
    let armement = if borne_ns <= maintenant_ns {
        Armement { tsc: plancher_tsc.ok_or(Refus::CibleInconnue)?, due: true }
    } else {
        let cible = cible_tsc.ok_or(Refus::CibleInconnue)?;
        if cible > tsc_maintenant {
            Armement { tsc: cible, due: false }
        } else {
            // Ecart entre l'horloge monotone et le TSC de CE coeur : la cible
            // est passee alors que la borne ne l'est pas. Le plancher garde la
            // meme cadence qu'une echeance due.
            Armement { tsc: plancher_tsc.ok_or(Refus::CibleInconnue)?, due: true }
        }
    };
    if armement.tsc <= tsc_maintenant {
        return Err(Refus::DejaPasse);
    }
    if armement.tsc >= arme_tsc {
        return Err(Refus::QuantumAvant);
    }
    Ok(armement)
}

/// L'echeance TSC a armer avant `hlt`, ou `None` pour laisser le quantum.
///
/// `borne_ns` : la plus proche echeance de sommeil connue (borne inferieure ;
/// `JAMAIS` pendant un balayage) ; `maintenant_ns` / `tsc_maintenant` :
/// l'heure ; `arme_tsc` : ce que le minuteur local porte deja (le quantum) ;
/// `cible_tsc` : `tsc_de(borne_ns)` ; `plancher_tsc` :
/// `tsc_de(maintenant_ns + MARGE_NS)` ; `voulu` : la politique.
pub fn decide(
    voulu: bool,
    borne_ns: u64,
    maintenant_ns: u64,
    tsc_maintenant: u64,
    arme_tsc: u64,
    cible_tsc: Option<u64>,
    plancher_tsc: Option<u64>,
) -> Option<u64> {
    decide_motif(voulu, borne_ns, maintenant_ns, tsc_maintenant, arme_tsc, cible_tsc, plancher_tsc)
        .ok()
        .map(|a| a.tsc)
}

/// La meme decision pour un minuteur local en mode PERIODIQUE (QEMU TCG n'a
/// pas TSC-deadline) : le compte a ecrire pour tirer a l'echeance, si elle
/// precede ce qui reste du compte courant. `comptes_par_ms` : la frequence du
/// minuteur apres son diviseur. Le quantum est restaure au declenchement.
///
/// Experience OFF (`reveil-precis periodique on`) : elle garde l'ancien refus
/// dans la marge, que BOUCHAUD_REVEIL_DU_V1 ne change que pour le TSC-deadline.
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
