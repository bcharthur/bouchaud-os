//! Le reveil a l'echeance exacte d'un coeur au repos (BOUCHAUD_REVEIL_PRECIS_V1).

#[path = "../../src/kernel/time/reveil_precis.rs"]
mod reveil_precis;

use reveil_precis::{decide, decide_compte, tsc_de, JAMAIS, MARGE_NS};

const HZ: u64 = 2_000_000_000; // 2 GHz : 2 cycles par ns
const BOOT: u64 = 1_000_000;

fn t(ns: u64) -> u64 {
    tsc_de(ns, BOOT, HZ).unwrap()
}

#[test]
fn conversion_inverse_de_l_horloge_monotone() {
    assert_eq!(tsc_de(0, BOOT, HZ), Some(BOOT));
    assert_eq!(tsc_de(1_000, BOOT, HZ), Some(BOOT + 2_000));
    assert_eq!(tsc_de(5, BOOT, 0), None, "frequence inconnue : rien a armer");
    assert_eq!(tsc_de(u64::MAX, u64::MAX, HZ), None, "debordement");
    // La conversion inverse de monotonic_ns : (tsc - boot) * 1e9 / hz.
    let ns = 123_456_789u64;
    let back = ((t(ns) - BOOT) as u128 * 1_000_000_000 / HZ as u128) as u64;
    assert_eq!(back, ns);
}

#[test]
fn une_echeance_avant_le_quantum_est_armee() {
    let maintenant = 10_000_000;
    let borne = maintenant + 700_000; // dans 0,7 ms
    let quantum = t(maintenant + 4_000_000);
    assert_eq!(decide(true, borne, maintenant, t(maintenant), quantum, Some(t(borne))), Some(t(borne)));
}

#[test]
fn le_quantum_plus_proche_reste_arme() {
    let maintenant = 10_000_000;
    let borne = maintenant + 6_000_000;
    let quantum = t(maintenant + 4_000_000);
    assert_eq!(decide(true, borne, maintenant, t(maintenant), quantum, Some(t(borne))), None);
}

#[test]
fn rien_sans_echeance_ni_pendant_un_balayage() {
    let maintenant = 10_000_000;
    let quantum = t(maintenant + 4_000_000);
    assert_eq!(decide(true, JAMAIS, maintenant, t(maintenant), quantum, None), None);
    assert_eq!(decide(true, 0, maintenant, t(maintenant), quantum, Some(BOOT)), None);
}

#[test]
fn une_echeance_due_ou_trop_proche_n_est_pas_armee() {
    // Due : le balayage du prochain passage la servira ; l'armer ferait
    // tirer le minuteur aussitot, en boucle si la borne est perimee.
    let maintenant = 10_000_000;
    let quantum = t(maintenant + 4_000_000);
    for borne in [maintenant - 1, maintenant, maintenant + MARGE_NS] {
        assert_eq!(decide(true, borne, maintenant, t(maintenant), quantum, Some(t(borne))), None, "borne {borne}");
    }
    let borne = maintenant + MARGE_NS + 1;
    assert!(decide(true, borne, maintenant, t(maintenant), quantum, Some(t(borne))).is_some());
}

#[test]
fn un_tsc_cible_deja_passe_n_est_pas_arme() {
    // Ecart entre les TSC des coeurs : la cible convertie peut etre deja
    // passee sur CE coeur meme si la borne est dans le futur.
    let maintenant = 10_000_000;
    let borne = maintenant + 500_000;
    let quantum = t(maintenant + 4_000_000);
    assert_eq!(decide(true, borne, maintenant, t(borne) + 10, quantum, Some(t(borne))), None);
}

#[test]
fn la_politique_desactivee_n_arme_jamais() {
    let maintenant = 10_000_000;
    let borne = maintenant + 700_000;
    let quantum = t(maintenant + 4_000_000);
    assert_eq!(decide(false, borne, maintenant, t(maintenant), quantum, Some(t(borne))), None);
}

// -- mode periodique (QEMU TCG) ------------------------------------------------

const PAR_MS: u64 = 62_586; // ~62,6 MHz apres le diviseur 16

#[test]
fn periodique_une_echeance_avant_la_fin_du_compte_est_armee() {
    let maintenant = 10_000_000;
    // 0,7 ms -> 43 811 comptes (arrondi superieur) ; il en reste 250 000.
    assert_eq!(decide_compte(true, maintenant + 700_000, maintenant, 250_000, PAR_MS), Some(43_811));
}

#[test]
fn periodique_le_quantum_plus_proche_reste() {
    let maintenant = 10_000_000;
    assert_eq!(decide_compte(true, maintenant + 3_000_000, maintenant, 100_000, PAR_MS), None);
}

#[test]
fn periodique_rien_sans_echeance_due_ou_sans_frequence() {
    let maintenant = 10_000_000;
    assert_eq!(decide_compte(true, JAMAIS, maintenant, 250_000, PAR_MS), None);
    assert_eq!(decide_compte(true, maintenant, maintenant, 250_000, PAR_MS), None);
    assert_eq!(decide_compte(true, maintenant + MARGE_NS, maintenant, 250_000, PAR_MS), None);
    assert_eq!(decide_compte(true, maintenant + 700_000, maintenant, 250_000, 0), None);
    assert_eq!(decide_compte(false, maintenant + 700_000, maintenant, 250_000, PAR_MS), None);
}

#[test]
fn periodique_le_compte_ne_tire_jamais_avant_l_echeance() {
    // Arrondi superieur : compte / frequence >= delai.
    let maintenant = 0;
    for delai in [20_001u64, 333_333, 999_999, 1_500_007] {
        let c = decide_compte(true, maintenant + delai, maintenant, u32::MAX, PAR_MS).unwrap() as u128;
        assert!(c * 1_000_000 >= delai as u128 * PAR_MS as u128, "delai {delai}");
    }
}
