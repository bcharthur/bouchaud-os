//! Le reveil a l'echeance exacte d'un coeur au repos (BOUCHAUD_REVEIL_PRECIS_V1).

#[path = "../../src/kernel/time/reveil_precis.rs"]
mod reveil_precis;

use reveil_precis::{decide, decide_compte, decide_motif, tsc_de, Armement, Refus, JAMAIS, MARGE_NS};

const HZ: u64 = 2_000_000_000; // 2 GHz : 2 cycles par ns
const BOOT: u64 = 1_000_000;

fn t(ns: u64) -> u64 {
    tsc_de(ns, BOOT, HZ).unwrap()
}

/// `plancher_tsc` tel que l'appelant le calcule.
fn p(maintenant: u64) -> Option<u64> {
    tsc_de(maintenant + MARGE_NS, BOOT, HZ)
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
fn conversion_ne_programme_jamais_avant_l_echeance_monotone() {
    // Le TSC calibre n'est pas necessairement un nombre entier de GHz.
    // Verifier l'horloge effective a la cible, puis au cycle precedent :
    // on veut le PREMIER cycle ou le sommeil est reellement echu.
    for hz in [10_000_000u64, 2_300_000_001, 3_579_545_123] {
        for ns in [1u64, 7, 999, 1_000_001, 123_456_789] {
            let cycles = tsc_de(ns, BOOT, hz).unwrap() - BOOT;
            let horloge = |c: u64| c as u128 * 1_000_000_000 / hz as u128;
            assert!(horloge(cycles) >= ns as u128, "tir avant l'echeance : hz={hz} ns={ns}");
            assert!(horloge(cycles - 1) < ns as u128, "retard evitable : hz={hz} ns={ns}");
        }
    }
}

#[test]
fn une_echeance_avant_le_quantum_est_armee() {
    let maintenant = 10_000_000;
    let borne = maintenant + 700_000; // dans 0,7 ms
    let quantum = t(maintenant + 4_000_000);
    assert_eq!(decide(true, borne, maintenant, t(maintenant), quantum, Some(t(borne)), p(maintenant)), Some(t(borne)));
}

#[test]
fn le_quantum_plus_proche_reste_arme() {
    let maintenant = 10_000_000;
    let borne = maintenant + 6_000_000;
    let quantum = t(maintenant + 4_000_000);
    assert_eq!(decide(true, borne, maintenant, t(maintenant), quantum, Some(t(borne)), p(maintenant)), None);
}

#[test]
fn rien_sans_echeance_ni_pendant_un_balayage() {
    let maintenant = 10_000_000;
    let quantum = t(maintenant + 4_000_000);
    assert_eq!(decide(true, JAMAIS, maintenant, t(maintenant), quantum, None, p(maintenant)), None);
    assert_eq!(decide(true, 0, maintenant, t(maintenant), quantum, Some(BOOT), p(maintenant)), None);
}

#[test]
fn une_echeance_dans_la_marge_est_armee_exactement() {
    // BOUCHAUD_REVEIL_DU_V1. Avant : refusee, et le coeur dormait jusqu'au
    // tic suivant (run 39308093631 : repos p95 851-955 us sous `on`).
    let maintenant = 10_000_000;
    let quantum = t(maintenant + 4_000_000);
    for borne in [maintenant + 1, maintenant + 5_000, maintenant + MARGE_NS] {
        assert_eq!(
            decide_motif(true, borne, maintenant, t(maintenant), quantum, Some(t(borne)), p(maintenant)),
            Ok(Armement { tsc: t(borne), due: false }),
            "borne {borne}"
        );
    }
}

#[test]
fn une_echeance_due_est_armee_a_une_marge_jamais_en_boucle() {
    // Due au moment d'armer (le temps a passe depuis le balayage) : servie
    // au plus une marge plus tard. Une borne perimee ne fait tirer le
    // minuteur qu'une fois par marge.
    let maintenant = 10_000_000;
    let quantum = t(maintenant + 4_000_000);
    for borne in [1, maintenant - 1_000_000, maintenant - 1, maintenant] {
        let a = decide_motif(true, borne, maintenant, t(maintenant), quantum, Some(t(borne)), p(maintenant)).unwrap();
        assert_eq!(a, Armement { tsc: t(maintenant + MARGE_NS), due: true }, "borne {borne}");
        assert!(a.tsc > t(maintenant), "jamais dans le passe : pas de tir immediat en boucle");
    }
}

#[test]
fn un_tsc_cible_deja_passe_est_arme_au_plancher() {
    // Ecart entre l'horloge monotone et le TSC de CE coeur : la cible
    // convertie est passee alors que la borne ne l'est pas. Avant : refus, et
    // le reveil attendait le tic. Desormais la cadence d'une echeance due.
    let maintenant = 10_000_000;
    let borne = maintenant + 500_000;
    let quantum = t(maintenant + 4_000_000);
    let tm = t(borne) + 10;
    let plancher = Some(tm + 40_000);
    assert_eq!(
        decide_motif(true, borne, maintenant, tm, quantum, Some(t(borne)), plancher),
        Ok(Armement { tsc: tm + 40_000, due: true })
    );
    // Plancher lui-meme passe : horloges incoherentes, rien d'arme.
    assert_eq!(
        decide_motif(true, borne, maintenant, tm, quantum, Some(t(borne)), Some(tm)),
        Err(Refus::DejaPasse)
    );
}

#[test]
fn une_echeance_due_ne_depasse_pas_le_quantum() {
    // Le quantum plus proche que la marge reste arme : il tirera plus tot.
    let maintenant = 10_000_000;
    let quantum = t(maintenant + 5_000);
    assert_eq!(
        decide_motif(true, maintenant - 1, maintenant, t(maintenant), quantum, Some(t(maintenant - 1)), p(maintenant)),
        Err(Refus::QuantumAvant)
    );
}

#[test]
fn la_politique_desactivee_n_arme_jamais() {
    let maintenant = 10_000_000;
    let borne = maintenant + 700_000;
    let quantum = t(maintenant + 4_000_000);
    assert_eq!(decide(false, borne, maintenant, t(maintenant), quantum, Some(t(borne)), p(maintenant)), None);
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

#[test]
fn chaque_refus_porte_son_motif() {
    let maintenant = 1_000_000_000;
    let quantum = t(maintenant + 4_000_000);
    let borne = maintenant + 1_000_000;
    let tm = t(maintenant);
    let pl = p(maintenant);
    assert_eq!(decide_motif(false, borne, maintenant, tm, quantum, Some(t(borne)), pl), Err(Refus::Politique));
    assert_eq!(decide_motif(true, JAMAIS, maintenant, tm, quantum, None, pl), Err(Refus::SansBorne));
    assert_eq!(decide_motif(true, borne, maintenant, tm, quantum, None, pl), Err(Refus::CibleInconnue));
    assert_eq!(decide_motif(true, maintenant, maintenant, tm, quantum, Some(tm), None), Err(Refus::CibleInconnue));
    assert_eq!(decide_motif(true, borne, maintenant, t(borne) + 1, quantum, Some(t(borne)), Some(t(borne))), Err(Refus::DejaPasse));
    let loin = maintenant + 9_000_000;
    assert_eq!(decide_motif(true, loin, maintenant, tm, quantum, Some(t(loin)), pl), Err(Refus::QuantumAvant));
    assert_eq!(decide_motif(true, borne, maintenant, tm, quantum, Some(t(borne)), pl), Ok(Armement { tsc: t(borne), due: false }));
}

#[test]
fn decide_est_le_motif_sans_le_motif() {
    let maintenant = 1_000_000_000;
    let quantum = t(maintenant + 4_000_000);
    for borne in [0, maintenant, maintenant + 5_000, maintenant + 30_000, maintenant + 3_000_000, maintenant + 8_000_000, JAMAIS] {
        let c = if borne == JAMAIS { None } else { Some(t(borne)) };
        assert_eq!(decide(true, borne, maintenant, t(maintenant), quantum, c, p(maintenant)),
                   decide_motif(true, borne, maintenant, t(maintenant), quantum, c, p(maintenant)).ok().map(|a| a.tsc),
                   "borne {borne}");
    }
}

#[test]
fn l_echeance_propre_sert_quand_la_borne_globale_est_en_balayage() {
    // BOUCHAUD_REVEIL_ECHEANCE_PROPRE_V1 : pendant le balayage d'un autre
    // coeur, la borne globale vaut JAMAIS ; la tache installee ici garde la
    // sienne.
    use reveil_precis::borne_du_coeur;
    assert_eq!(borne_du_coeur(JAMAIS, 5_000_000), 5_000_000);
    assert_eq!(borne_du_coeur(0, 5_000_000), 5_000_000);
    assert_eq!(borne_du_coeur(3_000_000, 5_000_000), 3_000_000, "la plus proche des deux");
    assert_eq!(borne_du_coeur(7_000_000, 5_000_000), 5_000_000);
    assert_eq!(borne_du_coeur(7_000_000, 0), 7_000_000, "aucune tache installee");
    assert_eq!(borne_du_coeur(JAMAIS, 0), JAMAIS);
    assert_eq!(borne_du_coeur(0, 0), JAMAIS);
}

#[test]
fn coeur_occupe_borne_interactive() {
    // BOUCHAUD_REVEIL_PRECIS_OCCUPE_V1 : la borne, ou une marge si elle est due.
    use reveil_precis::echeance_occupee;
    let maintenant = 10_000_000;
    assert_eq!(echeance_occupee(JAMAIS, maintenant), None);
    assert_eq!(echeance_occupee(0, maintenant), None);
    assert_eq!(echeance_occupee(maintenant + 700_000, maintenant), Some(maintenant + 700_000));
    assert_eq!(echeance_occupee(maintenant, maintenant), Some(maintenant + MARGE_NS), "due : une marge, pas un tir immediat");
    assert_eq!(echeance_occupee(1, maintenant), Some(maintenant + MARGE_NS));
}
#[test]
fn interruptions_precoces_ne_consomment_pas_le_quantum() {
    use reveil_precis::{avant_quantum, reste_compte};
    let quantum = 4_000_000;
    // PIT wakes HLT at 1, 2 and 3 ms; old cancellation extended it to 7 ms.
    for ms in 1..4 {
        let now = ms * 1_000_000;
        assert!(avant_quantum(now, quantum));
        assert_eq!(reste_compte(quantum, now, 1000), ((4 - ms) * 1000) as u32);
    }
    assert!(!avant_quantum(quantum, quantum));
    assert!(!avant_quantum(0, 0));
    assert_eq!(reste_compte(quantum, quantum + 10, 1000), 1);
    assert_eq!(reste_compte(1001, 0, 1000), 2); // never round a deadline down
}
