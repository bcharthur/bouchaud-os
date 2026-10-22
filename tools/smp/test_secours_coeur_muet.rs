//! Preuve hote de BOUCHAUD_SECOURS_COEUR_MUET_V1 : le module de PRODUCTION
//! `src/kernel/scheduler/secours.rs`, inclus tel quel.
//!
//! Le defaut : le fil `console`, fil noyau, en file sur le coeur que le banc
//! masquait 3 s (KVM SMP4, run 39667691144) -- personne ne pouvait le prendre,
//! le chien de garde l'a pris pour mort. Ces tests disent quand un coeur est
//! muet, quand une tache peut en etre relogee, et ou.

#[path = "../../src/kernel/scheduler/secours.rs"]
mod secours;

use secours::{choisit_accueil, coeur_muet, secours_permis, Accueil, SECOURS_SEUIL_NS};

const MS: u64 = 1_000_000;

#[test]
fn un_coeur_au_repos_n_est_jamais_muet() {
    assert!(!coeur_muet(0, 10_000 * MS, SECOURS_SEUIL_NS));
}

#[test]
fn le_quantum_et_les_points_cooperatifs_restent_sous_le_seuil() {
    // Un occupant installe depuis un quantum d'AP (4 ms) ou vingt points
    // cooperatifs (1 ms chacun) n'est pas muet.
    let t = 1_000 * MS;
    assert!(!coeur_muet(t - 4 * MS, t, SECOURS_SEUIL_NS));
    assert!(!coeur_muet(t - SECOURS_SEUIL_NS, t, SECOURS_SEUIL_NS));
    assert!(coeur_muet(t - SECOURS_SEUIL_NS - 1, t, SECOURS_SEUIL_NS));
}

#[test]
fn le_cas_du_banc_verrou_suspendu_est_secouru() {
    // Le banc tient le coeur 3 s ; le fil y attend depuis 30 ms.
    let t = 10_000 * MS;
    assert!(secours_permis(t - 1_000 * MS, t - 30 * MS, t, SECOURS_SEUIL_NS));
}

#[test]
fn une_tache_tout_juste_prete_n_est_pas_relogee() {
    // Occupant ancien, mais la tache vient d'arriver : le quantum la servira.
    let t = 10_000 * MS;
    assert!(!secours_permis(t - 1_000 * MS, t - 2 * MS, t, SECOURS_SEUIL_NS));
    // Pas prete du tout (horodatage nul).
    assert!(!secours_permis(t - 1_000 * MS, 0, t, SECOURS_SEUIL_NS));
}

#[test]
fn un_occupant_recent_n_est_pas_un_coeur_muet() {
    // La tache attend depuis longtemps, mais le coeur commute : il vient
    // d'elire quelqu'un. Ce n'est pas un coeur muet, c'est de la contention.
    let t = 10_000 * MS;
    assert!(!secours_permis(t - 3 * MS, t - 500 * MS, t, SECOURS_SEUIL_NS));
}

#[test]
fn horloges_incoherentes_ne_secourent_pas() {
    // Un occupant ou une tache « dans le futur » (lecture croisee) : jamais.
    let t = 10_000 * MS;
    assert!(!secours_permis(t + 5 * MS, t - 500 * MS, t, SECOURS_SEUIL_NS));
    assert!(!secours_permis(t - 500 * MS, t + 5 * MS, t, SECOURS_SEUIL_NS));
}

fn accueil(permis: bool, inactif: bool, muet: bool, attente: usize) -> Accueil {
    Accueil { permis, inactif, muet, attente }
}

#[test]
fn jamais_le_donneur_ni_un_coeur_interdit_ou_muet() {
    let coeurs = [
        accueil(true, true, false, 0),  // donneur
        accueil(false, true, false, 0), // interdit par l'affinite
        accueil(true, true, true, 0),   // muet lui aussi
        accueil(true, false, false, 7), // vivant, charge
    ];
    assert_eq!(choisit_accueil(0, &coeurs), Some(3));
}

#[test]
fn un_coeur_au_repos_d_abord_puis_la_file_la_plus_courte() {
    let coeurs = [
        accueil(true, false, false, 0),
        accueil(true, false, false, 1),
        accueil(true, true, false, 5),
        accueil(true, false, false, 0),
    ];
    assert_eq!(choisit_accueil(1, &coeurs), Some(2));
    let sans_repos = [
        accueil(true, false, false, 3),
        accueil(true, false, false, 1),
        accueil(true, false, false, 1),
        accueil(true, false, false, 0),
    ];
    assert_eq!(choisit_accueil(3, &sans_repos), Some(1));
}

#[test]
fn personne_pour_accueillir() {
    // SMP1, ou tous les autres coeurs muets / interdits : pas de secours.
    assert_eq!(choisit_accueil(0, &[accueil(true, true, false, 0)]), None);
    let coeurs = [accueil(true, false, false, 0), accueil(true, false, true, 0), accueil(false, true, false, 0)];
    assert_eq!(choisit_accueil(0, &coeurs), None);
}

/// Propriete, graines : l'accueil retenu respecte toujours les trois
/// interdits, et aucun coeur permis ne le bat au rang.
#[test]
fn property_accueil_respecte_les_interdits_et_le_rang() {
    let mut x: u64 = std::env::var("BOUCHAUD_PROP_SEED")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(|s| s.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
        .unwrap_or(0x5EC0_0125);
    let mut suivant = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    for _ in 0..50_000 {
        let n = 1 + (suivant() % 16) as usize;
        let donneur = (suivant() % n as u64) as usize;
        let coeurs: Vec<Accueil> = (0..n)
            .map(|_| {
                let r = suivant();
                accueil(r & 1 != 0, r & 2 != 0, r & 4 != 0, ((r >> 8) % 6) as usize)
            })
            .collect();
        match choisit_accueil(donneur, &coeurs) {
            Some(c) => {
                assert_ne!(c, donneur);
                assert!(coeurs[c].permis && !coeurs[c].muet);
                for (i, o) in coeurs.iter().enumerate() {
                    if i == donneur || !o.permis || o.muet {
                        continue;
                    }
                    assert!((!coeurs[c].inactif, coeurs[c].attente) <= (!o.inactif, o.attente));
                }
            }
            None => {
                assert!(coeurs
                    .iter()
                    .enumerate()
                    .all(|(i, o)| i == donneur || !o.permis || o.muet));
            }
        }
    }
}
