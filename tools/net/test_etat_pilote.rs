//! LA MACHINE A ETATS DU PILOTE, A L'HOTE.
//!
//! # Le defaut, releve en photo le 18 septembre 2026
//!
//! Apres une reprise de degre quatre, l'ecran de la TRIGKEY affichait :
//!
//! ```text
//! net.nic.e1000     Erreur          carte non pilotee
//! net.nic.rtl8168   Indisponible    absente de ce materiel
//! ```
//!
//! Le RTL8168 est soude sur cette machine et venait de recevoir
//! soixante-quatre trames ; la e1000 n'y a jamais existe. Les deux lignes
//! etaient fausses, parce qu'un seul booleen -- `is_ready()` -- portait
//! quatre questions independantes.
//!
//! Ce banc fixe la regle qui compte : RIEN ne ramene une carte a `Absent`.

#[path = "../../src/drivers/network/etat_pilote.rs"]
mod etat_pilote;

use etat_pilote::{resume, transition_permise, Etat};

#[test]
fn rien_ne_ramene_une_carte_a_absente() {
    // LA REGLE CENTRALE. Une puce soudee ne disparait pas parce que notre
    // code a renonce : c'est exactement la transition que l'ancienne reprise
    // de degre quatre effectuait en posant `READY = false` et `MMIO = 0`.
    for depuis in [Etat::Detecte, Etat::Lie, Etat::Pret, Etat::Reprise, Etat::Echec] {
        assert!(
            !transition_permise(depuis, Etat::Absent),
            "« {} » ne doit pas pouvoir redevenir absent",
            depuis.nom()
        );
    }
    // Seul le neant mene au neant.
    assert!(transition_permise(Etat::Absent, Etat::Absent));
}

#[test]
fn un_echec_de_reprise_garde_la_carte_presente_et_attachee() {
    // « Si la reinitialisation echoue : Failed/Offline, jamais absent. »
    assert!(Etat::Echec.presente());
    assert!(Etat::Echec.attache());
    assert!(!Etat::Echec.en_service());
    let (mot, raison) = resume(Etat::Echec, false);
    assert_eq!(mot, "panne");
    assert_ne!(raison, "absente de ce materiel");
}

#[test]
fn une_reprise_ne_detache_pas_le_pilote() {
    assert!(Etat::Reprise.presente());
    assert!(Etat::Reprise.attache());
    // Mais elle n'autorise pas a emettre : les anneaux sont en cours de
    // reprogrammation.
    assert!(!Etat::Reprise.en_service());
}

#[test]
fn seule_une_carte_absente_se_dit_absente() {
    let (_, raison) = resume(Etat::Absent, false);
    assert_eq!(raison, "absente de ce materiel");
    for etat in [Etat::Detecte, Etat::Lie, Etat::Pret, Etat::Reprise, Etat::Echec] {
        let (_, raison) = resume(etat, false);
        assert_ne!(
            raison, "absente de ce materiel",
            "« {} » ne doit pas se dire absent",
            etat.nom()
        );
    }
}

#[test]
fn le_cycle_nominal_est_permis() {
    assert!(transition_permise(Etat::Absent, Etat::Detecte));
    assert!(transition_permise(Etat::Detecte, Etat::Lie));
    assert!(transition_permise(Etat::Lie, Etat::Pret));
    assert!(transition_permise(Etat::Pret, Etat::Reprise));
    assert!(transition_permise(Etat::Reprise, Etat::Pret));
}

#[test]
fn un_echec_n_est_pas_definitif() {
    // On a le droit de retenter : une carte qui a rate un reset peut en
    // reussir un autre quand le lien revient.
    assert!(transition_permise(Etat::Echec, Etat::Reprise));
    assert!(transition_permise(Etat::Echec, Etat::Pret));
    assert!(Etat::Echec.reprise_possible());
}

#[test]
fn on_ne_saute_pas_l_attachement() {
    // Une puce detectee mais sans pilote ne peut pas etre « prete ».
    assert!(!transition_permise(Etat::Detecte, Etat::Pret));
    assert!(!transition_permise(Etat::Absent, Etat::Pret));
    assert!(!Etat::Detecte.attache());
}

#[test]
fn le_lien_ne_change_pas_l_etat_du_pilote() {
    // Cable debranche : le pilote reste pret, c'est le LIEN qui est bas.
    // Quatre faits distincts, et non un seul.
    assert!(Etat::Pret.en_service());
    let (mot_haut, _) = resume(Etat::Pret, true);
    let (mot_bas, raison_bas) = resume(Etat::Pret, false);
    assert_eq!(mot_haut, "actif");
    assert_eq!(mot_bas, "attente");
    assert_eq!(raison_bas, "lien bas");
}
