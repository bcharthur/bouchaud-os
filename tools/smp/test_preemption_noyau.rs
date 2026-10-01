//! La decision de preempter un fil noyau depuis une interruption.
//!
//! BOUCHAUD_PREEMPTION_NOYAU_SURE_V1
//!
//! Compile `src/kernel/scheduler/preemption_noyau.rs` -- le code meme que le
//! noyau execute au tic -- et verifie EXHAUSTIVEMENT, sur les 256
//! combinaisons de la demande et des sept conditions de surete :
//!
//!   S1 aucune preemption sans demande ciblee (le noyau n'est pas preemptible
//!      en son sein ; le quantum noyau est retire) ;
//!   S2 aucune preemption hors contexte sur : section non preemptable, verrou
//!      simple, verrou lockdep, lecture du registre tenue, shootdown TLB en
//!      vol, sortie en cours, interruptions ouvertes (l'IRQ imbriquee) ;
//!   S3 demande ciblee + contexte sur => preemption ;
//!   S4 chaque condition, seule, suffit a refuser.
//!
//! Lance par `tools/ci/run_host_tests.sh`.

#[path = "../../src/kernel/scheduler/preemption_noyau.rs"]
mod preemption_noyau;

use preemption_noyau::{contexte_sur, decide, Contexte};

fn sur(ciblee: bool) -> Contexte {
    Contexte {
        demande_ciblee: ciblee,
        preempt_count: 0,
        verrous_simples: 0,
        profondeur_lockdep: 0,
        lectures_registre: 0,
        shootdown_en_vol: false,
        sortie_en_cours: false,
        interruptions_ouvertes: false,
    }
}

#[test]
fn s1_s2_s3_exhaustif() {
    let mut accordees = 0u32;
    for masque in 0u32..(1 << 8) {
        let b = |i: u32| masque & (1 << i) != 0;
        let c = Contexte {
            demande_ciblee: b(0),
            preempt_count: b(1) as u32,
            verrous_simples: 2 * b(2) as u32,
            profondeur_lockdep: b(3) as u32,
            lectures_registre: b(4) as u32,
            shootdown_en_vol: b(5),
            sortie_en_cours: b(6),
            interruptions_ouvertes: b(7),
        };
        let attendu_sur = masque >> 1 == 0;
        assert_eq!(contexte_sur(&c), attendu_sur, "contexte_sur, masque {masque:#010b}");
        let obtenu = decide(&c);
        assert_eq!(obtenu, b(0) && attendu_sur, "decide, masque {masque:#010b}");
        if obtenu {
            accordees += 1;
            assert!(b(0), "S1 : accordee sans demande ciblee");
            assert!(attendu_sur, "S2 : accordee hors contexte sur");
        }
    }
    assert_eq!(accordees, 1, "une seule combinaison accorde : demande + tout sur");
}

#[test]
fn s4_chaque_condition_suffit_a_refuser() {
    let refus: [(&str, fn(&mut Contexte)); 7] = [
        ("section non preemptable", |c| c.preempt_count = 1),
        ("verrou simple", |c| c.verrous_simples = 1),
        ("verrou lockdep", |c| c.profondeur_lockdep = 3),
        ("lecture du registre", |c| c.lectures_registre = 1),
        ("shootdown en vol", |c| c.shootdown_en_vol = true),
        ("sortie en cours", |c| c.sortie_en_cours = true),
        ("IRQ imbriquee possible (IF ouvert)", |c| c.interruptions_ouvertes = true),
    ];
    assert!(decide(&sur(true)));
    for (nom, f) in refus {
        let mut c = sur(true);
        f(&mut c);
        assert!(!decide(&c), "{nom} : la demande ciblee doit etre refusee");
    }
}

#[test]
fn le_cas_mesure_lecteur_du_registre() {
    // `services-metrics` parcourant la table sous garde de lecture, une
    // demande ciblee pendante sur son coeur : jamais commute.
    let mut c = sur(true);
    c.lectures_registre = 1;
    assert!(!decide(&c));
    assert!(!decide(&sur(false)), "sans demande, un fil noyau n'est jamais coupe");
}
