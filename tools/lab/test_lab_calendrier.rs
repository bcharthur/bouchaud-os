//! Une echeance de checkpoint ratee coute-t-elle encore quarante-cinq secondes ?
//!
//! # Le releve que ces epreuves defendent
//!
//! TRIGKEY, campagne `1179cdd` :
//!
//! ```text
//! session reelle     ~395 s
//! records persistes  ~193 s
//! checkpoints        0
//! fin                false
//! ```
//!
//! Deux cents secondes de session n'ont jamais atteint le support, et pas un
//! point de reprise en pres de sept minutes. Le fil `services-metrics`
//! tournait normalement : il n'y a pas eu de famine, et sa priorite n'est pas
//! en cause.
//!
//! L'echeance etait avancee AVANT de savoir si le support suivait. Au premier
//! passage -- cent millisecondes apres l'amorcage, bien avant qu'une cle USB
//! soit enumeree -- elle sautait a quarante-cinq secondes, et chaque echeance
//! ratee ensuite en coutait quarante-cinq de plus.
//!
//! Lance par `tools/ci/run_host_tests.sh`.

#[path = "../../src/kernel/debug/lab/calendrier.rs"]
mod calendrier;

use calendrier::{
    Calendrier, Decision, Raison, ECHECS_AVANT_CADENCE_NORMALE, PERIODE_MS,
    REPORT_COURT_MS, REPORT_PALIER_MS,
};

/// Rejoue une session : rend les instants (ms) ou un checkpoint est pose.
///
/// `support(t)` dit si la cle repond a l'instant `t`. La boucle appelle le
/// calendrier a dix hertz, comme le fait `services-metrics` -- qui, lui,
/// tourne a dix hertz et se borne lui-meme.
fn session(duree_ms: u64, support: impl Fn(u64) -> bool) -> Vec<u64> {
    let mut c = Calendrier::nouveau();
    let mut poses = Vec::new();
    let mut t = 0u64;
    while t <= duree_ms {
        if let Decision::Pose = c.consulte(t, support(t)) {
            poses.push(t);
            c.pose_reussie(t);
        }
        t += 100;
    }
    poses
}

// ===========================================================================
// LE DEFAUT CORRIGE
// ===========================================================================

#[test]
fn un_support_qui_arrive_a_cinq_secondes_est_servi_vers_cinq_secondes() {
    // L'ENONCE, MOT POUR MOT. Avant : t=45..47. Apres : t=5..7.
    let poses = session(60_000, |t| t >= 5_000);
    let premier = *poses.first().expect("un checkpoint doit avoir lieu");
    assert!(
        (5_000..=7_000).contains(&premier),
        "premier checkpoint a {premier} ms ; attendu entre 5 000 et 7 000",
    );
    assert!(
        premier < 45_000,
        "c'est exactement le defaut corrige : une echeance ratee ne doit plus \
couter quarante-cinq secondes",
    );
}

#[test]
fn l_ancien_comportement_aurait_attendu_quarante_cinq_secondes() {
    // LA CONTRADICTION, CHIFFREE. On rejoue l'ancien enchainement -- avancer
    // l'echeance AVANT de regarder le support -- sur le meme scenario.
    let mut prochain = 0u64;
    let mut premier = None;
    let mut t = 0u64;
    while t <= 60_000 {
        if t >= prochain {
            prochain = t + PERIODE_MS; // <-- la ligne qui coutait tout
            if t >= 5_000 && premier.is_none() {
                premier = Some(t);
            }
        }
        t += 100;
    }
    assert_eq!(
        premier,
        Some(45_000),
        "l'ancien calendrier ne pouvait pas poser avant la quarante-cinquieme \
seconde, quelle que soit l'heure d'arrivee du support",
    );
}

#[test]
fn une_echeance_ratee_ne_se_consomme_pas() {
    let mut c = Calendrier::nouveau();
    // Due tout de suite, support absent : report court, pas de periode.
    let d = c.consulte(0, false);
    assert_eq!(
        d,
        Decision::Reporte { raison: Raison::SansSupport, prochain_ms: REPORT_COURT_MS },
    );
    assert_eq!(c.echeance_ms(), REPORT_COURT_MS);
    assert!(c.echeance_ms() < PERIODE_MS);
}

#[test]
fn le_report_reste_court_et_ne_derive_pas_vers_la_periode() {
    // UN REPORT QUI DOUBLERAIT jusqu'a quarante-cinq secondes ramenerait le
    // defaut, en plus lent : une cle branchee a la trentieme seconde
    // attendrait de nouveau la soixante-quinzieme.
    let mut c = Calendrier::nouveau();
    let mut t = 0u64;
    for _ in 0..200 {
        match c.consulte(t, false) {
            Decision::Reporte { prochain_ms, .. } => {
                assert!(
                    prochain_ms - t <= REPORT_PALIER_MS,
                    "report de {} ms a t={t}",
                    prochain_ms - t,
                );
                t = prochain_ms;
            }
            Decision::Attendre { restant_ms } => t += restant_ms,
            Decision::Pose => panic!("sans support, on ne pose pas"),
        }
    }
    assert!(t < 500_000, "deux cents reports ne doivent pas couvrir des heures");
}

#[test]
fn une_cle_branchee_tres_tard_est_servie_tout_de_suite() {
    // Le corollaire du palier : meme a la trois-centieme seconde, l'attente
    // supplementaire vaut deux secondes, pas quarante-cinq.
    let poses = session(320_000, |t| t >= 300_000);
    let premier = *poses.first().expect("un checkpoint doit avoir lieu");
    assert!(
        premier <= 302_000,
        "premier checkpoint a {premier} ms apres un support arrive a 300 000",
    );
}

// ===========================================================================
// LA CADENCE NORMALE, UNE FOIS LE SUPPORT LA
// ===========================================================================

#[test]
fn apres_une_pose_la_cadence_redevient_celle_de_croisiere() {
    let mut c = Calendrier::nouveau();
    assert_eq!(c.consulte(0, true), Decision::Pose);
    c.pose_reussie(0);
    assert_eq!(c.echeance_ms(), PERIODE_MS);
    assert_eq!(
        c.consulte(PERIODE_MS - 1, true),
        Decision::Attendre { restant_ms: 1 },
    );
    assert_eq!(c.consulte(PERIODE_MS, true), Decision::Pose);
}

#[test]
fn au_dela_de_quarante_cinq_secondes_il_y_a_au_moins_un_checkpoint() {
    // CRITERE PHYSIQUE. Support present du debut a la fin.
    let poses = session(46_000, |_| true);
    assert!(
        !poses.is_empty(),
        "plus de 45 s de session doivent produire au moins un checkpoint",
    );
}

#[test]
fn au_dela_de_quatre_vingt_dix_secondes_il_y_en_a_au_moins_deux() {
    // CRITERE PHYSIQUE.
    let poses = session(91_000, |_| true);
    assert!(
        poses.len() >= 2,
        "plus de 90 s doivent produire au moins deux checkpoints -- obtenu {poses:?}",
    );
}

#[test]
fn quatre_vingt_dix_secondes_avec_un_support_tardif_tiennent_aussi_le_critere() {
    // La correction ne doit pas se payer ailleurs : un support arrive a la
    // cinquieme seconde doit rendre DEUX checkpoints avant la
    // quatre-vingt-dixieme, pas un seul.
    let poses = session(91_000, |t| t >= 5_000);
    assert!(poses.len() >= 2, "obtenu {poses:?}");
}

#[test]
fn un_support_qui_disparait_ne_fait_pas_perdre_la_cadence() {
    // Cle retiree entre la vingtieme et la soixantieme seconde. A son retour,
    // le checkpoint en retard se pose tout de suite -- pas quarante-cinq
    // secondes plus tard.
    let poses = session(120_000, |t| !(20_000..60_000).contains(&t));
    assert!(poses.len() >= 2, "obtenu {poses:?}");
    let apres_retour = poses.iter().find(|&&t| t >= 60_000);
    assert!(
        apres_retour.is_some_and(|&t| t <= 62_000),
        "au retour du support, le rattrapage doit etre immediat -- {poses:?}",
    );
}

// ===========================================================================
// L'ECHEC D'ECRITURE
// ===========================================================================

#[test]
fn un_checkpoint_qui_echoue_est_reessaye_de_peu_puis_espace() {
    // Un support qui repond mais n'ecrit pas est peut-etre en train de
    // mourir. Trois chances rapprochees, puis la cadence normale : marteler
    // une cle defaillante ne la repare pas.
    let mut c = Calendrier::nouveau();
    let mut t = 0u64;
    for essai in 1..=ECHECS_AVANT_CADENCE_NORMALE {
        assert_eq!(c.consulte(t, true), Decision::Pose, "essai {essai}");
        let Decision::Reporte { raison, prochain_ms } = c.pose_echouee(t) else {
            panic!("un echec reporte toujours");
        };
        assert_eq!(raison, Raison::EchecEcriture);
        assert_eq!(prochain_ms - t, REPORT_PALIER_MS, "essai {essai}");
        t = prochain_ms;
    }
    // La quatrieme fois, on espace.
    assert_eq!(c.consulte(t, true), Decision::Pose);
    let Decision::Reporte { prochain_ms, .. } = c.pose_echouee(t) else {
        panic!()
    };
    assert_eq!(prochain_ms - t, PERIODE_MS);
}

#[test]
fn un_succes_apres_des_echecs_efface_l_ardoise() {
    let mut c = Calendrier::nouveau();
    c.consulte(0, true);
    c.pose_echouee(0);
    c.consulte(REPORT_PALIER_MS, true);
    c.pose_echouee(REPORT_PALIER_MS);
    let t = 2 * REPORT_PALIER_MS;
    assert_eq!(c.consulte(t, true), Decision::Pose);
    c.pose_reussie(t);
    assert_eq!(c.poses(), 1);
    assert_eq!(c.echeance_ms(), t + PERIODE_MS);
    // Et un echec suivant repart des reports courts, pas de la cadence longue.
    let t2 = t + PERIODE_MS;
    assert_eq!(c.consulte(t2, true), Decision::Pose);
    let Decision::Reporte { prochain_ms, .. } = c.pose_echouee(t2) else { panic!() };
    assert_eq!(prochain_ms - t2, REPORT_PALIER_MS);
}

// ===========================================================================
// LE CONTRAT D'APPEL
// ===========================================================================

#[test]
fn consulter_ne_consomme_rien_tant_qu_on_ne_rend_pas_compte() {
    // UN CHECKPOINT QUI N'A PAS EU LIEU DOIT AVOIR LIEU. Si l'appelant se
    // fait interrompre entre `consulte` et le compte rendu, l'echeance reste
    // atteinte et la tentative se represente -- c'est le bon defaut.
    let mut c = Calendrier::nouveau();
    assert_eq!(c.consulte(0, true), Decision::Pose);
    assert_eq!(c.consulte(0, true), Decision::Pose, "sans compte rendu, ca insiste");
    assert_eq!(c.echeance_ms(), 0);
    assert_eq!(c.poses(), 0);
}

#[test]
fn un_calendrier_neuf_est_du_tout_de_suite() {
    // C'est ce qui permet a un support branche tard d'etre servi tot : la
    // question se pose des la premiere seconde, et « pas encore » ne coute
    // plus une periode entiere.
    let mut c = Calendrier::nouveau();
    assert_eq!(c.echeance_ms(), 0);
    assert_eq!(c.consulte(0, true), Decision::Pose);
}

#[test]
fn forcer_l_echeance_rend_le_checkpoint_du_immediatement() {
    let mut c = Calendrier::nouveau();
    c.consulte(0, true);
    c.pose_reussie(0);
    assert!(matches!(c.consulte(10, true), Decision::Attendre { .. }));
    c.force();
    assert_eq!(c.consulte(10, true), Decision::Pose);
}

#[test]
fn le_temps_qui_ne_progresse_pas_ne_pose_pas_en_boucle() {
    // Une horloge figee a zero est une horloge non calibree. Le calendrier ne
    // doit pas y voir une invitation a poser mille checkpoints.
    let mut c = Calendrier::nouveau();
    let mut poses = 0;
    for _ in 0..1_000 {
        if let Decision::Pose = c.consulte(0, true) {
            poses += 1;
            c.pose_reussie(0);
        }
    }
    assert_eq!(poses, 1, "une horloge figee autorise UN checkpoint, pas mille");
}

#[test]
fn les_reports_consecutifs_se_comptent() {
    // Un statut qui dit « zero checkpoint » sans dire « et trente reports
    // faute de support » laisse croire a une boite noire morte.
    let mut c = Calendrier::nouveau();
    let mut t = 0u64;
    for attendu in 1..=10u32 {
        let Decision::Reporte { prochain_ms, .. } = c.consulte(t, false) else {
            panic!("sans support on reporte")
        };
        assert_eq!(c.reports_consecutifs(), attendu);
        t = prochain_ms;
    }
    assert_eq!(c.consulte(t, true), Decision::Pose);
    c.pose_reussie(t);
    assert_eq!(c.reports_consecutifs(), 0, "un succes efface l'ardoise");
}
