//! Test hote du prefiltre du vecteur timer/IPI (P13 P2/P4).
#[path = "../../src/kernel/scheduler/echeances.rs"]
mod echeances;
use echeances::{Echeances, JAMAIS};

#[test]
fn ipi_avant_echeance_et_nouvelle_publication() {
    let e = Echeances::neuve();
    assert!(!e.doit_balayer(100));
    e.arme(400);
    for now in [0, 100, 399] {
        assert!(!e.doit_balayer(now));
    }
    assert!(e.doit_balayer(400));
    e.arme(250);
    assert!(e.doit_balayer(300));
}
#[test]
fn refus_sous_verrou_ne_consomme_pas_la_borne() {
    let e = Echeances::neuve();
    e.arme(250);
    let preemption_sure = false;
    let transition_ouverte = true;
    assert!(e.doit_balayer(300));
    assert!(!(e.doit_balayer(300) && preemption_sure && !transition_ouverte));
    assert_eq!(e.borne(), 250);
}
#[test]
fn cpu_secondaire_ne_prend_pas_le_scan_deja_revendique() {
    let e = Echeances::neuve();
    e.arme(100);
    assert!(e.commence_balayage(100));
    assert_eq!(e.borne(), JAMAIS);
    assert!(!e.doit_balayer(100));
    e.arme(105);
    e.recale(500);
    assert_eq!(e.borne(), 105);
    assert!(!e.doit_balayer(104));
    assert!(e.doit_balayer(105));
}
#[test]
fn reveil_hlt_suivi_d_un_autre_sommeil() {
    let e = Echeances::neuve();
    e.arme(80);
    assert!(e.doit_balayer(80));
    assert!(e.commence_balayage(80));
    e.recale(JAMAIS);
    assert!(!e.doit_balayer(81));
    e.arme(90);
    assert!(e.doit_balayer(90));
}

#[path = "../../src/kernel/scheduler/preemption_noyau.rs"]
mod preemption_noyau;

#[test]
fn garde_smp_garde_toutes_les_exclusions_existantes() {
    use preemption_noyau::{contexte_sur, decide, Contexte};
    let sur = Contexte {
        demande_ciblee: true,
        preempt_count: 0,
        verrous_simples: 0,
        profondeur_lockdep: 0,
        lectures_registre: 0,
        shootdown_en_vol: false,
        sortie_en_cours: false,
        interruptions_ouvertes: false,
    };
    assert!(contexte_sur(&sur));
    assert!(decide(&sur));
    assert!(!decide(&Contexte { demande_ciblee: false, ..sur }));
    assert!(!contexte_sur(&Contexte { preempt_count: 1, ..sur }));
    assert!(!contexte_sur(&Contexte { verrous_simples: 1, ..sur }));
    assert!(!contexte_sur(&Contexte { profondeur_lockdep: 1, ..sur }));
    assert!(!contexte_sur(&Contexte { lectures_registre: 1, ..sur }));
    assert!(!contexte_sur(&Contexte { shootdown_en_vol: true, ..sur }));
    assert!(!contexte_sur(&Contexte { sortie_en_cours: true, ..sur }));
    assert!(!contexte_sur(&Contexte { interruptions_ouvertes: true, ..sur }));
}
