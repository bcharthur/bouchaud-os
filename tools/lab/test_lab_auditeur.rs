//! L'auditeur reconnait-il le releve TRIGKEY, et se tait-il sur une machine saine ?
//!
//! # Le releve que ces epreuves rejouent
//!
//! ```text
//! tour 1 :  rx_packets 0 -> 64,  rx_cur 0 -> 63 -> 0
//! ensuite : rx_packets=64  rx_cur=0  desc_nic=64  desc_cpu=0   (151 s)
//!           isr_rx_ok croissant   rx_missed=0   chip_cmd=RX_ENB|TX_ENB
//! ```
//!
//! Aucun bit d'erreur, aucune trame manquee, le moteur arme. Rien dans cet
//! etat ne se declare en panne tout seul : c'est une CONJONCTION de compteurs
//! qui le dit, et une conjonction se prouve par arithmetique.
//!
//! Le risque symetrique est le faux positif. Un auditeur qui accuse une
//! machine saine fait ignorer ses verdicts, et la panne suivante passera avec
//! le bruit. La moitie de ces epreuves defend donc le SILENCE.
//!
//! Lance par `tools/ci/run_host_tests.sh`.

#[path = "../../src/kernel/debug/lab/auditeur.rs"]
mod auditeur;

use auditeur::{
    Auditeur, Observation, Verdict, CADENCE_ANOMALIE_HZ, CADENCE_NORMALE_HZ,
    FENETRE_ANOMALIE_NS, SEUIL_BB_STALL_NS, SEUIL_STALL_NS, SEUIL_TOUR2_NS,
};

const RX_ENB: u8 = 0x08;
const TX_ENB: u8 = 0x04;
const SECONDE: u64 = 1_000_000_000;

/// Une machine saine : lien haut, moteur arme, anneau intact, rien a dire.
fn saine(t_ns: u64) -> Observation {
    Observation {
        t_ns,
        lien: true,
        chip_cmd: RX_ENB | TX_ENB,
        desc_materiel: 60,
        desc_processeur: 4,
        bb_storage_ready: true,
        ..Observation::default()
    }
}

/// L'etat fige du releve physique, a l'instant `t_ns`, apres `tours` de
/// progression d'`isr_rx_ok`.
///
/// Tous les compteurs de reception sont a l'arret ; seule la carte continue
/// d'annoncer. C'est la signature exacte, et elle ne contient aucune erreur.
fn trigkey_fige(t_ns: u64, isr_ticks: u64) -> Observation {
    Observation {
        t_ns,
        lien: true,
        rx_paquets: 64,
        rx_cur: 0,
        rx_tours_cpu: 1,
        rx_rendus_tour1: 64,
        rx_rendus_tour2: 0,
        rx_rearmes_tour1: 64,
        rx_reutilises_tour2: 0,
        desc_materiel: 64,
        desc_processeur: 0,
        rx_own_rendus: 64,
        isr_rx_ok: 100 + isr_ticks * 7,
        rx_missed: 0,
        chip_cmd: RX_ENB | TX_ENB,
        bb_storage_ready: true,
        bb_records_ram: 100,
        bb_records_persistes: 100,
        ..Observation::default()
    }
}

fn a_un<F: Fn(&Verdict) -> bool>(r: &auditeur::Rapport, p: F) -> bool {
    r.verdicts().any(|v| p(&v))
}

fn est_stall(v: &Verdict) -> bool {
    matches!(v, Verdict::RxDmaStall { .. })
}
fn est_tour2(v: &Verdict) -> bool {
    matches!(v, Verdict::SecondTourAbsent { .. })
}
fn est_invariant(v: &Verdict) -> bool {
    matches!(v, Verdict::AnneauInvariant { .. })
}
fn est_bb(v: &Verdict) -> bool {
    matches!(v, Verdict::BlackboxPersistenceStall { .. })
}
fn est_moteur(v: &Verdict) -> bool {
    matches!(v, Verdict::MoteurRxArrete { .. })
}

// ===========================================================================
// LE SILENCE
// ===========================================================================

#[test]
fn une_machine_saine_ne_produit_aucun_verdict() {
    // L'EPREUVE LA PLUS IMPORTANTE DU FICHIER. Un auditeur qui accuse une
    // machine saine fait ignorer ses verdicts, et la panne suivante passera
    // avec le bruit.
    let mut a = Auditeur::nouveau();
    let mut paquets = 0u64;
    for tour in 0..300u64 {
        let mut obs = saine(tour * SECONDE);
        paquets += 12;
        obs.rx_paquets = paquets;
        obs.isr_rx_ok = paquets;
        obs.rx_own_rendus = paquets;
        obs.desc_processeur = (tour % 8) as u32;
        obs.rx_tours_cpu = paquets / 64;
        obs.rx_rendus_tour1 = paquets.min(64);
        obs.rx_rendus_tour2 = paquets.saturating_sub(64);
        obs.bb_records_ram = tour * 3;
        obs.bb_records_persistes = tour * 3;
        let r = a.examine(&obs);
        assert!(r.is_empty(), "tour {tour} : {:?}", r.verdicts().collect::<Vec<_>>());
    }
    assert_eq!(a.cadence_hz(), CADENCE_NORMALE_HZ);
}

#[test]
fn un_reseau_calme_n_est_pas_une_carte_en_panne() {
    // Rien n'entre ET la carte n'annonce rien : c'est un reseau muet, pas une
    // carte qui ment. Sans cette distinction, toute machine posee sur un
    // commutateur sans trafic se declarerait en panne au bout de trois
    // secondes.
    let mut a = Auditeur::nouveau();
    for tour in 0..120u64 {
        let mut obs = saine(tour * SECONDE);
        obs.rx_paquets = 500;
        obs.isr_rx_ok = 500;
        obs.rx_own_rendus = 500;
        obs.desc_processeur = 0;
        let r = a.examine(&obs);
        assert!(r.is_empty(), "tour {tour} : un reseau calme ne s'accuse pas");
    }
}

#[test]
fn sans_lien_on_n_accuse_personne() {
    // Cable debranche : `chip_cmd` peut tomber, les compteurs figent. Rien de
    // cela n'est une panne de carte.
    let mut a = Auditeur::nouveau();
    for tour in 0..60u64 {
        let obs = Observation {
            t_ns: tour * SECONDE,
            lien: false,
            chip_cmd: 0,
            ..Observation::default()
        };
        assert!(a.examine(&obs).is_empty(), "tour {tour}");
    }
}

#[test]
fn un_pilote_qui_ne_remplit_rien_ne_declenche_rien() {
    // Des zeros constants. Les regles ne comparent que des ECARTS et des
    // conjonctions : aucune ne peut se satisfaire de l'immobilite totale.
    let mut a = Auditeur::nouveau();
    for tour in 0..200u64 {
        let obs = Observation { t_ns: tour * SECONDE, ..Observation::default() };
        assert!(a.examine(&obs).is_empty(), "tour {tour}");
    }
}

// ===========================================================================
// LE RELEVE TRIGKEY
// ===========================================================================

#[test]
fn le_releve_trigkey_fige_se_declare_en_panne() {
    // L'ACCUSATION. On rejoue la signature exacte : compteurs de reception
    // figes, `isr_rx_ok` qui monte, tout l'anneau au materiel.
    let mut a = Auditeur::nouveau();
    let mut vu_stall = false;
    let mut vu_tour2 = false;
    for tour in 0..20u64 {
        let r = a.examine(&trigkey_fige(tour * SECONDE, tour));
        vu_stall |= a_un(&r, est_stall);
        vu_tour2 |= a_un(&r, est_tour2);
    }
    assert!(vu_stall, "AUDIT_RX_DMA_STALL doit etre rendu");
    assert!(vu_tour2, "AUDIT_SECOND_TOUR_ABSENT doit etre rendu");
}

#[test]
fn le_stall_demande_les_trois_conditions_et_pas_deux() {
    // `rx_paquets` immobile seul : reseau calme.
    // `isr_rx_ok` qui monte seul avec des descripteurs qui reviennent : trafic.
    // Les deux ensemble SANS descripteur rendu : la carte ment.
    let mut a = Auditeur::nouveau();

    // Cas 1 : isr monte ET des descripteurs reviennent -> rien.
    for tour in 0..10u64 {
        let mut obs = trigkey_fige(tour * SECONDE, tour);
        obs.rx_own_rendus = 64 + tour; // le materiel restitue
        assert!(!a_un(&a.examine(&obs), est_stall), "tour {tour}");
    }

    // Cas 2 : plus rien ne revient -> accusation.
    let mut a = Auditeur::nouveau();
    let mut vu = false;
    for tour in 0..10u64 {
        vu |= a_un(&a.examine(&trigkey_fige(tour * SECONDE, tour)), est_stall);
    }
    assert!(vu);
}

#[test]
fn le_stall_ne_se_declare_pas_avant_son_seuil() {
    let mut a = Auditeur::nouveau();
    // Premier tour : pose du temoin, rien ne peut encore etre juge.
    assert!(a.examine(&trigkey_fige(0, 0)).is_empty());
    // Juste avant le seuil : toujours rien.
    let r = a.examine(&trigkey_fige(SEUIL_STALL_NS - 1, 1));
    assert!(!a_un(&r, est_stall), "le seuil n'est pas atteint");
    // Au seuil : l'accusation tombe.
    let r = a.examine(&trigkey_fige(SEUIL_STALL_NS, 2));
    assert!(a_un(&r, est_stall));
}

#[test]
fn le_second_tour_absent_se_compte_depuis_le_premier_bouclage() {
    // La regle ne peut pas partir de l'amorcage : un anneau qui n'a pas encore
    // boucle n'a rien a prouver.
    let mut a = Auditeur::nouveau();

    // Cent secondes de premier tour en cours : aucun reproche.
    for tour in 0..100u64 {
        let mut obs = saine(tour * SECONDE);
        obs.rx_paquets = tour;
        obs.isr_rx_ok = tour;
        obs.rx_own_rendus = tour;
        obs.rx_tours_cpu = 0;
        assert!(!a_un(&a.examine(&obs), est_tour2), "tour {tour}");
    }

    // L'anneau boucle a t=100 s. Le compte commence LA.
    let base = 100 * SECONDE;
    assert!(!a_un(&a.examine(&trigkey_fige(base, 0)), est_tour2));
    assert!(
        !a_un(&a.examine(&trigkey_fige(base + SEUIL_TOUR2_NS - 1, 1)), est_tour2),
        "le seuil se compte depuis le bouclage, pas depuis l'amorcage",
    );
    let r = a.examine(&trigkey_fige(base + SEUIL_TOUR2_NS, 2));
    assert!(a_un(&r, est_tour2));
    let trouve = r.verdicts().find(|v| est_tour2(v)).unwrap();
    match trouve {
        Verdict::SecondTourAbsent { rendus_tour1, rendus_tour2, attente_ns } => {
            assert_eq!(rendus_tour1, 64);
            assert_eq!(rendus_tour2, 0);
            assert_eq!(attente_ns, SEUIL_TOUR2_NS);
        }
        autre => panic!("{autre:?}"),
    }
}

#[test]
fn un_second_tour_qui_demarre_efface_le_reproche() {
    // Le critere physique de reussite est `RX_RENDUS_TOUR2 > 0`. Un seul
    // descripteur de second tour, et l'anneau circule.
    let mut a = Auditeur::nouveau();
    for tour in 0..10u64 {
        a.examine(&trigkey_fige(tour * SECONDE, tour));
    }
    let mut obs = trigkey_fige(20 * SECONDE, 20);
    obs.rx_rendus_tour2 = 1;
    obs.rx_paquets = 65;
    assert!(!a_un(&a.examine(&obs), est_tour2));
    // Et la panne qui revient est un fait NEUF : elle se resignale.
    let mut vu = false;
    for tour in 21..40u64 {
        let mut obs = trigkey_fige(tour * SECONDE, tour);
        obs.rx_paquets = 65;
        vu |= a_un(&a.examine(&obs), est_tour2);
    }
    assert!(vu, "une panne qui revient doit se redire");
}

#[test]
fn un_verdict_ne_se_repete_pas_a_chaque_tour() {
    // LA PANNE DURE CENT CINQUANTE SECONDES. La signaler a chaque tour
    // noierait l'anneau -- cent cinquante emissions identiques -- et ferait
    // perdre les evenements qui l'entourent. On signale la TRANSITION.
    let mut a = Auditeur::nouveau();
    let mut stalls = 0usize;
    let mut tour2 = 0usize;
    for tour in 0..151u64 {
        let r = a.examine(&trigkey_fige(tour * SECONDE, tour));
        stalls += r.verdicts().filter(|v| est_stall(v)).count();
        tour2 += r.verdicts().filter(|v| est_tour2(v)).count();
    }
    assert_eq!(stalls, 1, "un seul AUDIT_RX_DMA_STALL pour une panne continue");
    assert_eq!(tour2, 1, "un seul AUDIT_SECOND_TOUR_ABSENT");
}

// ===========================================================================
// L'ANNEAU ET LE MOTEUR
// ===========================================================================

#[test]
fn un_invariant_casse_est_detecte_et_nomme() {
    let mut a = Auditeur::nouveau();
    let mut obs = saine(SECONDE);
    obs.invariant_casse = true;
    obs.invariant_code = 2; // eor-double
    obs.rx_cur = 17;
    let r = a.examine(&obs);
    assert!(a_un(&r, est_invariant));
    let trouve = r.verdicts().find(|v| est_invariant(v)).unwrap();
    match trouve {
        Verdict::AnneauInvariant { code, rx_cur } => {
            assert_eq!(code, 2);
            assert_eq!(rx_cur, 17);
        }
        autre => panic!("{autre:?}"),
    }
}

#[test]
fn un_invariant_repare_rearme_la_detection() {
    let mut a = Auditeur::nouveau();
    let mut casse = saine(SECONDE);
    casse.invariant_casse = true;
    casse.invariant_code = 1;
    assert!(a_un(&a.examine(&casse), est_invariant));
    assert!(!a_un(&a.examine(&casse), est_invariant), "pas deux fois de suite");
    assert!(!a_un(&a.examine(&saine(3 * SECONDE)), est_invariant));
    assert!(a_un(&a.examine(&casse), est_invariant), "une rechute se redit");
}

#[test]
fn un_moteur_de_reception_eteint_est_detecte() {
    let mut a = Auditeur::nouveau();
    let mut obs = saine(SECONDE);
    obs.chip_cmd = TX_ENB; // RxEnb tombe
    assert!(a_un(&a.examine(&obs), est_moteur));
}

#[test]
fn une_carte_pas_encore_programmee_n_est_pas_un_moteur_eteint() {
    // `chip_cmd = 0` avant la programmation du materiel. Accuser la, c'est
    // accuser l'amorcage.
    let mut a = Auditeur::nouveau();
    let mut obs = saine(SECONDE);
    obs.chip_cmd = 0;
    assert!(!a_un(&a.examine(&obs), est_moteur));
}

// ===========================================================================
// LA BOITE NOIRE
// ===========================================================================

#[test]
fn la_persistance_figee_pendant_que_la_ram_avance_est_detectee() {
    // L'ENONCE PHYSIQUE : session reelle 395 s, records persistes 193 s. Deux
    // cents secondes d'ecart ne devaient pas passer inapercues.
    let mut a = Auditeur::nouveau();
    let mut vu = false;
    for tour in 0..20u64 {
        let mut obs = saine(tour * SECONDE);
        obs.rx_paquets = tour * 10;
        obs.isr_rx_ok = tour * 10;
        obs.rx_own_rendus = tour * 10;
        obs.bb_records_ram = 100 + tour * 5; // la RAM avance
        obs.bb_records_persistes = 100; // le support est fige
        vu |= a_un(&a.examine(&obs), est_bb);
    }
    assert!(vu, "AUDIT_BLACKBOX_PERSISTENCE_STALL doit etre rendu");
}

#[test]
fn une_boite_noire_au_repos_n_est_pas_une_boite_noire_en_panne() {
    // Ni la RAM ni le support n'avancent : rien n'est en retard. C'est une
    // machine tranquille, et l'accuser serait un faux positif permanent.
    let mut a = Auditeur::nouveau();
    for tour in 0..60u64 {
        let mut obs = saine(tour * SECONDE);
        obs.rx_paquets = tour;
        obs.isr_rx_ok = tour;
        obs.rx_own_rendus = tour;
        obs.bb_records_ram = 100;
        obs.bb_records_persistes = 100;
        assert!(!a_un(&a.examine(&obs), est_bb), "tour {tour}");
    }
}

#[test]
fn la_persistance_ne_s_accuse_pas_avant_cinq_secondes() {
    let mut a = Auditeur::nouveau();
    let mut obs = saine(0);
    obs.bb_records_ram = 100;
    obs.bb_records_persistes = 100;
    a.examine(&obs);

    let mut obs = saine(SEUIL_BB_STALL_NS - 1);
    obs.bb_records_ram = 200;
    obs.bb_records_persistes = 100;
    assert!(!a_un(&a.examine(&obs), est_bb));

    let mut obs = saine(SEUIL_BB_STALL_NS);
    obs.bb_records_ram = 300;
    obs.bb_records_persistes = 100;
    assert!(a_un(&a.examine(&obs), est_bb));
}

#[test]
fn une_persistance_qui_repart_efface_l_ardoise() {
    let mut a = Auditeur::nouveau();
    let mut obs = saine(0);
    obs.bb_records_ram = 100;
    obs.bb_records_persistes = 100;
    a.examine(&obs);
    let mut obs = saine(SEUIL_BB_STALL_NS);
    obs.bb_records_ram = 300;
    obs.bb_records_persistes = 100;
    assert!(a_un(&a.examine(&obs), est_bb));

    let mut obs = saine(SEUIL_BB_STALL_NS + SECONDE);
    obs.bb_records_ram = 310;
    obs.bb_records_persistes = 250; // le support repart
    assert!(!a_un(&a.examine(&obs), est_bb));
}

// ===========================================================================
// LA CADENCE
// ===========================================================================

#[test]
fn la_cadence_monte_sur_anomalie_et_retombe_d_elle_meme() {
    // UNE PANNE DURABLE NE DOIT PAS TENIR DIX HERTZ POUR TOUJOURS. La panne
    // RTL8168 dure cent cinquante et une secondes, et l'accompagner a dix
    // hertz fausserait la mesure d'ordonnancement qu'on prend par ailleurs.
    let mut a = Auditeur::nouveau();
    assert_eq!(a.cadence_hz(), CADENCE_NORMALE_HZ);

    let mut obs = saine(SECONDE);
    obs.invariant_casse = true;
    let r = a.examine(&obs);
    assert_eq!(r.cadence_hz, CADENCE_ANOMALIE_HZ);
    assert!(r.cadence_changee);

    // Pendant la fenetre, on reste rapide.
    let r = a.examine(&saine(SECONDE + FENETRE_ANOMALIE_NS - 1));
    assert_eq!(r.cadence_hz, CADENCE_ANOMALIE_HZ);
    assert!(!r.cadence_changee, "rester rapide n'est pas un changement");

    // Passe la fenetre, elle retombe seule.
    let r = a.examine(&saine(SECONDE + FENETRE_ANOMALIE_NS));
    assert_eq!(r.cadence_hz, CADENCE_NORMALE_HZ);
    assert!(r.cadence_changee);
}

#[test]
fn une_anomalie_reporte_la_fin_de_la_fenetre_rapide() {
    let mut a = Auditeur::nouveau();
    let mut obs = saine(SECONDE);
    obs.invariant_casse = true;
    a.examine(&obs);

    // Une seconde anomalie a mi-fenetre : la fenetre repart de la.
    let mut obs = saine(SECONDE + FENETRE_ANOMALIE_NS / 2);
    obs.invariant_casse = false;
    a.examine(&obs);
    let mut obs = saine(SECONDE + FENETRE_ANOMALIE_NS / 2 + 1);
    obs.invariant_casse = true;
    assert_eq!(a.examine(&obs).cadence_hz, CADENCE_ANOMALIE_HZ);

    let tard = SECONDE + FENETRE_ANOMALIE_NS / 2 + 1 + FENETRE_ANOMALIE_NS - 1;
    assert_eq!(a.examine(&saine(tard)).cadence_hz, CADENCE_ANOMALIE_HZ);
}

// ===========================================================================
// LA CAPTURE
// ===========================================================================

#[test]
fn seuls_les_verdicts_reseau_demandent_une_capture_materielle() {
    // Capturer soixante-dix evenements de registres RTL8168 pour une panne de
    // boite noire noierait l'anneau sans rien apprendre.
    assert!(Verdict::RxDmaStall { silence_ns: 0, isr_delta: 1, desc_processeur: 0 }
        .demande_capture());
    assert!(Verdict::SecondTourAbsent { rendus_tour1: 64, rendus_tour2: 0, attente_ns: 0 }
        .demande_capture());
    assert!(Verdict::AnneauInvariant { code: 1, rx_cur: 0 }.demande_capture());
    assert!(Verdict::MoteurRxArrete { chip_cmd: 4 }.demande_capture());
    assert!(!Verdict::BlackboxPersistenceStall {
        records_ram: 1,
        records_persistes: 0,
        fige_depuis_ns: 0,
    }
    .demande_capture());
}

#[test]
fn un_rapport_est_borne_et_ne_deborde_pas() {
    // Cinq regles, cinq places. Un rapport qui deborderait silencieusement
    // ferait disparaitre le verdict le plus grave -- celui qui arrive en
    // dernier n'etant pas le moins important.
    let mut a = Auditeur::nouveau();
    let mut obs = trigkey_fige(0, 0);
    obs.bb_records_ram = 100;
    obs.bb_records_persistes = 100;
    a.examine(&obs);

    let mut obs = trigkey_fige(10 * SECONDE, 10);
    obs.invariant_casse = true;
    obs.invariant_code = 3;
    obs.chip_cmd = TX_ENB;
    obs.bb_records_ram = 500;
    obs.bb_records_persistes = 100;
    let r = a.examine(&obs);
    assert!(r.len() <= auditeur::VERDICTS_MAX);
    assert_eq!(r.verdicts().count(), r.len());
    // Les cinq regles ont toutes de quoi parler : on les veut toutes.
    assert!(a_un(&r, est_stall));
    assert!(a_un(&r, est_tour2));
    assert!(a_un(&r, est_invariant));
    assert!(a_un(&r, est_moteur));
    assert!(a_un(&r, est_bb));
    assert_eq!(r.len(), 5);
}

#[test]
fn le_compteur_de_tours_avance_meme_sans_verdict() {
    // Un auditeur muet et un auditeur mort se ressemblent trop.
    let mut a = Auditeur::nouveau();
    for tour in 0..10u64 {
        a.examine(&saine(tour * SECONDE));
    }
    assert_eq!(a.tours(), 10);
}

#[test]
fn le_temps_qui_ne_progresse_pas_ne_declenche_rien() {
    // Une horloge non calibree rend zero. Tous les ecarts valent zero, donc
    // aucun seuil n'est franchi -- et surtout aucun ne l'est par debordement.
    let mut a = Auditeur::nouveau();
    for _ in 0..50 {
        let obs = trigkey_fige(0, 0);
        assert!(a.examine(&obs).is_empty());
    }
}
