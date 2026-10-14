//! Jusqu'ou un appelant interruptible attend le jeton serie
//! (BOUCHAUD_SERIE_ATTENTE_PROGRES_V1).
//!
//! Run 38603003639 (KVM SMP4) : une ligne emise sans le jeton apres une
//! seconde d'attente, au milieu de celle d'un autre. La borne portait sur
//! l'attente totale ; elle porte desormais sur l'absence de progres.

#[path = "../../src/drivers/serial/attente.rs"]
mod attente;

use attente::*;

const S: u64 = 1_000_000_000;

#[test]
fn faire_la_queue_derriere_des_emetteurs_qui_progressent_n_est_pas_une_panne() {
    // Une seconde d'attente, et 180 lignes sont sorties entre-temps.
    match a_l_echeance(101 * S, 100 * S, 7, 187) {
        Echeance::Renouvelee { echeance_ns, emissions_vues } => {
            assert_eq!(echeance_ns, 102 * S);
            assert_eq!(emissions_vues, 187);
        }
        autre => panic!("renouvelee attendue, {autre:?}"),
    }
}

#[test]
fn une_seconde_sans_aucune_ligne_c_est_un_detenteur_perdu() {
    assert_eq!(a_l_echeance(101 * S, 100 * S, 7, 7), Echeance::Renonce(Renonce::SansProgres));
}

#[test]
fn le_plafond_absolu_finit_l_attente_meme_si_d_autres_emettent() {
    assert_eq!(a_l_echeance(110 * S, 100 * S, 7, 9_999), Echeance::Renonce(Renonce::Plafond));
    // Juste avant le plafond : encore renouvelee.
    assert!(matches!(a_l_echeance(110 * S - 1, 100 * S, 7, 8), Echeance::Renouvelee { .. }));
}

#[test]
fn une_attente_renouvelee_finit_au_plus_au_plafond() {
    // D'autres emettent sans cesse : l'appelant renouvelle seconde apres
    // seconde, et renonce au plus tard au plafond absolu.
    let debut = 100 * S;
    let (mut maintenant, mut vues, mut emissions) = (debut + S, 0u64, 0u64);
    let mut tours = 0;
    loop {
        emissions += 50;
        match a_l_echeance(maintenant, debut, vues, emissions) {
            Echeance::Renouvelee { echeance_ns, emissions_vues } => {
                vues = emissions_vues;
                maintenant = echeance_ns;
                tours += 1;
                assert!(tours < 100, "attente infinie");
            }
            Echeance::Renonce(r) => {
                assert_eq!(r, Renonce::Plafond);
                assert!(maintenant - debut >= PLAFOND_ABSOLU_NS);
                break;
            }
        }
    }
    assert_eq!(tours, 9);
}

#[test]
fn le_compteur_qui_reboucle_est_un_progres() {
    // Vingt ans de lignes plus tard : une difference reste un progres.
    assert!(matches!(a_l_echeance(101 * S, 100 * S, u64::MAX, 0), Echeance::Renouvelee { .. }));
}
