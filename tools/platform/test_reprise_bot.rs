//! L'etat du transport Bulk-Only, et ce qu'on a le droit d'en attendre.
//!
//! Ce que ces epreuves defendent : le critere d'acceptation B -- une commande
//! de stockage qui expire ne tue NI le clavier NI l'ordonnanceur. La regle qui
//! le garantit tient en une phrase : aucune entree-sortie nouvelle avant que
//! la reprise n'ait rendu le materiel coherent, et un refus immediat -- sans
//! attente, sans verrou tenu -- quand elle n'y arrive pas.

#[path = "../../src/drivers/usb/reprise_bot.rs"]
mod bot;

use bot::{Etat, Incident, Phase, Transport, REPRISES_AVANT_HORS_SERVICE};

#[test]
fn un_transport_neuf_laisse_passer() {
    let t = Transport::neuf();
    assert_eq!(t.etat(), Etat::Pret);
    assert!(t.autorise_es());
    assert_eq!(t.releve().refus, 0);
}

#[test]
fn une_echeance_ferme_le_transport_immediatement() {
    // Le defaut d'avant : la commande suivante partait comme si de rien
    // n'etait, sur un anneau ou un TD abandonne attendait encore son
    // achevement -- qu'elle prenait alors pour le sien.
    let t = Transport::neuf();
    t.entre_en_phase(Phase::Donnees);
    assert_eq!(t.incident(Incident::Echeance, 3, 4, 1_000), Etat::Reprise);
    assert!(!t.autorise_es(), "rien ne passe avant la reprise");
    let r = t.releve();
    assert_eq!(r.etat, Etat::Reprise);
    assert_eq!(r.derniere_phase, Phase::Donnees);
    assert_eq!(r.dernier_slot, 3);
    assert_eq!(r.dernier_dci, 4);
    assert_eq!(r.echeances, 1);
    assert_eq!(r.refus, 1);
    assert_eq!(r.dernier_incident_ns, 1_000);
    assert!(!r.autorise());
}

#[test]
fn seule_une_echeance_laisse_un_td_derriere_elle() {
    // Un STALL ou un CSW refuse sont arrives AVEC leur evenement : le TD est
    // consomme. Reinitialiser le pointeur de file dans ces cas-la remettrait
    // a zero un anneau sain.
    assert!(Incident::Echeance.laisse_un_td());
    assert!(!Incident::PointArrete.laisse_un_td());
    assert!(!Incident::StatutInvalide.laisse_un_td());
    assert!(!Incident::PhaseIncoherente.laisse_un_td());
}

#[test]
fn une_reprise_reussie_rouvre_le_transport() {
    let t = Transport::neuf();
    t.entre_en_phase(Phase::Statut);
    t.incident(Incident::StatutInvalide, 1, 2, 500);
    assert!(t.commence_reprise());
    t.reprise_reussie();
    assert_eq!(t.etat(), Etat::Pret);
    assert!(t.autorise_es());
    let r = t.releve();
    assert_eq!(r.reprises, 1);
    assert_eq!(r.reprises_reussies, 1);
    assert_eq!(r.reprises_echouees, 0);
    assert_eq!(r.tentatives_en_cours, 0);
    assert_eq!(r.derniere_phase, Phase::Repos, "la phase repart de zero");
}

#[test]
fn la_reprise_ne_commence_pas_sur_un_transport_sain() {
    // Sinon le compteur de reprises monterait sans qu'aucune panne n'ait eu
    // lieu, et le chiffre qu'on lit dans l'archive ne voudrait plus rien dire.
    let t = Transport::neuf();
    assert!(!t.commence_reprise());
    assert_eq!(t.releve().reprises, 0);
}

#[test]
fn trop_d_echecs_mettent_le_transport_hors_service_et_pas_avant() {
    let t = Transport::neuf();
    t.incident(Incident::Echeance, 1, 2, 0);
    for tentative in 1..REPRISES_AVANT_HORS_SERVICE {
        assert!(t.commence_reprise());
        assert_eq!(
            t.reprise_echouee(),
            Etat::Reprise,
            "tentative {} ne doit pas encore condamner",
            tentative
        );
    }
    assert!(t.commence_reprise());
    assert_eq!(t.reprise_echouee(), Etat::HorsService);
    assert_eq!(t.etat(), Etat::HorsService);
    let r = t.releve();
    assert_eq!(r.reprises_echouees, REPRISES_AVANT_HORS_SERVICE as u64);
}

#[test]
fn hors_service_refuse_tout_de_suite_et_ne_se_reprend_plus_seul() {
    // C'est ce qui protege le clavier : un support mort coute un
    // `compare_exchange` et un compteur, pas une attente sous verrou.
    let t = Transport::neuf();
    t.incident(Incident::Echeance, 1, 2, 0);
    for _ in 0..REPRISES_AVANT_HORS_SERVICE {
        assert!(t.commence_reprise());
        t.reprise_echouee();
    }
    assert_eq!(t.etat(), Etat::HorsService);
    assert!(!t.autorise_es());
    assert!(!t.commence_reprise(), "on ne rouvre pas de soi-meme");
    // Un incident de plus ne le fait pas redescendre en Reprise.
    assert_eq!(
        t.incident(Incident::Echeance, 1, 2, 10),
        Etat::HorsService
    );
    assert_eq!(t.releve().refus, 1);
}

#[test]
fn une_reconfiguration_du_support_remet_le_transport_a_neuf() {
    // Un support retire puis rebranche repasse par l'enumeration. Sans cette
    // remise a zero, il resterait hors service pour le reste de la session --
    // et un debranchement accidentel couterait le stockage jusqu'au
    // redemarrage.
    let t = Transport::neuf();
    t.incident(Incident::Echeance, 1, 2, 0);
    for _ in 0..REPRISES_AVANT_HORS_SERVICE {
        t.commence_reprise();
        t.reprise_echouee();
    }
    assert_eq!(t.etat(), Etat::HorsService);
    t.remet_a_neuf();
    assert_eq!(t.etat(), Etat::Pret);
    assert!(t.autorise_es());
    assert_eq!(t.releve().tentatives_en_cours, 0);
    // Les compteurs d'histoire, eux, SURVIVENT : un support qui se remet a
    // neuf trois fois par minute est un support a remplacer, et c'est ce
    // chiffre-la qui le dit.
    assert_eq!(
        t.releve().reprises_echouees,
        REPRISES_AVANT_HORS_SERVICE as u64
    );
}

#[test]
fn une_reprise_reussie_apres_deux_echecs_remet_le_compteur_de_tentatives() {
    // Sans cette remise a zero, deux pannes espacees d'une heure se
    // cumuleraient et condamneraient un support parfaitement utilisable.
    let t = Transport::neuf();
    t.incident(Incident::Echeance, 1, 2, 0);
    t.commence_reprise();
    t.reprise_echouee();
    t.commence_reprise();
    t.reprise_echouee();
    t.commence_reprise();
    t.reprise_reussie();
    assert_eq!(t.etat(), Etat::Pret);

    t.incident(Incident::PhaseIncoherente, 1, 2, 100);
    t.commence_reprise();
    assert_eq!(
        t.reprise_echouee(),
        Etat::Reprise,
        "le compteur de tentatives doit etre reparti de zero"
    );
}

#[test]
fn chaque_etat_phase_et_incident_a_un_code_et_un_nom_stables() {
    for e in [Etat::Pret, Etat::Reprise, Etat::HorsService] {
        assert_eq!(Etat::depuis_code(e.code()), e);
        assert!(!e.nom().is_empty());
    }
    for p in [Phase::Repos, Phase::Commande, Phase::Donnees, Phase::Statut] {
        assert_eq!(Phase::depuis_code(p.code()), p);
        assert!(!p.nom().is_empty());
    }
    for i in [
        Incident::Echeance,
        Incident::PointArrete,
        Incident::StatutInvalide,
        Incident::PhaseIncoherente,
    ] {
        assert!(i.code() != 0);
        assert!(!i.nom().is_empty());
    }
    assert_eq!(Etat::depuis_code(99), Etat::Pret);
    assert_eq!(Phase::depuis_code(99), Phase::Repos);
}
