//! Ou passe le temps entre deux tours de scrutation HID.
//!
//! Ce que ces epreuves defendent : qu'un ecart de 638 ms designe UN
//! responsable et un seul. Trois hypotheses pour un chiffre, c'est zero
//! diagnostic -- et c'est exactement ce que la session physique du
//! 17 septembre a produit.

#[path = "../../src/drivers/usb/chrono_hid.rs"]
mod chrono;

use chrono::{ChronoHid, PROPRIETAIRES};

const MS: u64 = 1_000_000;

#[test]
fn un_chrono_neuf_n_accuse_personne() {
    let c = ChronoHid::neuf();
    let r = c.releve();
    assert_eq!(r.wake_to_run_max_us, 0);
    assert_eq!(r.run_to_lock_max_us, 0);
    assert_eq!(r.poll_body_max_us, 0);
    assert_eq!(r.lock_fail_total, 0);
    assert_eq!(r.lock_fail_streak_max, 0);
    assert_eq!(r.tours, 0);
}

#[test]
fn les_trois_intervalles_se_mesurent_separement() {
    let c = ChronoHid::neuf();
    // Echeance a 100 ms, reprise a 112 ms, verrou a 115 ms, sortie a 119 ms.
    c.note_reveil(100 * MS, 112 * MS);
    c.note_verrou_pris(112 * MS, 115 * MS);
    c.note_corps(115 * MS, 119 * MS);
    let r = c.releve();
    assert_eq!(r.wake_to_run_max_us, 12_000);
    assert_eq!(r.run_to_lock_max_us, 3_000);
    assert_eq!(r.poll_body_max_us, 4_000);
    assert_eq!(r.tours, 1);
}

#[test]
fn une_reprise_en_avance_ne_devient_pas_le_pire_de_la_session() {
    // `sleep_ticks` arrondit au tick : une reprise peut precéder l'echeance
    // calculee. Une soustraction naive rendrait dix-huit milliards d'annees,
    // qui resteraient le maximum pour toujours.
    let c = ChronoHid::neuf();
    c.note_reveil(100 * MS, 99 * MS);
    assert_eq!(c.releve().wake_to_run_max_us, 0);
    c.note_reveil(100 * MS, 105 * MS);
    assert_eq!(c.releve().wake_to_run_max_us, 5_000);
}

#[test]
fn chaque_maximum_ne_recule_jamais() {
    let c = ChronoHid::neuf();
    c.note_reveil(0, 50 * MS);
    c.note_verrou_pris(0, 40 * MS);
    c.note_corps(0, 30 * MS);
    // Des tours calmes ensuite : les maxima tiennent.
    for _ in 0..100 {
        c.note_reveil(0, 1 * MS);
        c.note_verrou_pris(0, 1 * MS);
        c.note_corps(0, 1 * MS);
    }
    let r = c.releve();
    assert_eq!(r.wake_to_run_max_us, 50_000);
    assert_eq!(r.run_to_lock_max_us, 40_000);
    assert_eq!(r.poll_body_max_us, 30_000);
    assert_eq!(r.tours, 101);
}

#[test]
fn le_responsable_est_celui_qui_domine() {
    // C'est cette regle qui decide du correctif. La laisser a la lecture d'un
    // tableau, c'est risquer de corriger la mauvaise moitie -- ce qui coute
    // une session physique.
    let c = ChronoHid::neuf();
    c.note_reveil(0, 100 * MS);
    c.note_verrou_pris(0, 10 * MS);
    c.note_corps(0, 5 * MS);
    assert_eq!(c.releve().responsable(), "ordonnanceur");

    let c = ChronoHid::neuf();
    c.note_reveil(0, 10 * MS);
    c.note_verrou_pris(0, 100 * MS);
    c.note_corps(0, 5 * MS);
    assert_eq!(c.releve().responsable(), "verrou");

    let c = ChronoHid::neuf();
    c.note_reveil(0, 10 * MS);
    c.note_verrou_pris(0, 5 * MS);
    c.note_corps(0, 100 * MS);
    assert_eq!(c.releve().responsable(), "chemin-hid");
}

#[test]
fn un_chrono_vierge_ne_designe_pas_un_coupable_au_hasard() {
    // Tout a zero : la regle doit rendre quelque chose de stable, et pas
    // varier selon l'ordre des comparaisons.
    assert_eq!(ChronoHid::neuf().releve().responsable(), "ordonnanceur");
}

#[test]
fn la_serie_de_refus_compte_et_se_remet_a_zero_sur_un_succes() {
    // Renoncer trois fois de suite n'est pas la meme chose que renoncer trois
    // fois dans la minute : c'est la SERIE qui dit si le verrou est tenu en
    // continu.
    let c = ChronoHid::neuf();
    for _ in 0..5 {
        c.note_echec_verrou(3, 0);
    }
    assert_eq!(c.releve().lock_fail_streak_max, 5);
    c.note_verrou_pris(0, 0);
    for _ in 0..2 {
        c.note_echec_verrou(3, 0);
    }
    let r = c.releve();
    assert_eq!(r.lock_fail_total, 7, "le total, lui, ne se remet pas a zero");
    assert_eq!(r.lock_fail_streak_max, 5, "la pire serie tient");
}

#[test]
fn les_refus_se_ventilent_par_proprietaire() {
    // « Le verrou etait pris » n'est pas un diagnostic ; « le systeme de
    // fichiers l'a pris quatre-vingts fois » en est un.
    let c = ChronoHid::neuf();
    c.note_echec_verrou(2, 0); // repli-hid
    c.note_echec_verrou(2, 0);
    c.note_echec_verrou(3, 0); // systeme de fichiers
    let r = c.releve();
    assert_eq!(r.lock_fail_owner[2], 2);
    assert_eq!(r.lock_fail_owner[3], 1);
    assert_eq!(r.lock_fail_owner[0], 0);
    assert_eq!(r.lock_fail_total, 3);
}

#[test]
fn un_code_de_proprietaire_hors_bornes_ne_deborde_pas() {
    // Le code vient d'un atomique partage : une valeur inattendue doit etre
    // rangee, pas faire paniquer la scrutation du clavier.
    let c = ChronoHid::neuf();
    c.note_echec_verrou(200, 0);
    assert_eq!(c.releve().lock_fail_owner[PROPRIETAIRES - 1], 1);
}

#[test]
fn la_famine_se_mesure_du_premier_refus_jusqu_au_succes() {
    // LE TROU QUE CETTE EPREUVE FERME
    //
    // Au premier banc, les trois intervalles totalisaient 31 ms pour un ecart
    // mesure de 144 ms. Les 113 manquantes etaient une serie de trente-cinq
    // refus : chaque tour refuse est COURT, donc `run_to_lock` restait petit
    // pendant que l'attente reelle explosait.
    let c = ChronoHid::neuf();
    c.note_echec_verrou(3, 100 * MS);
    for i in 1..35 {
        c.note_echec_verrou(3, (100 + i * 4) as u64 * MS);
    }
    assert_eq!(c.releve().lock_starve_max_us, 0, "la famine n'est pas encore fermee");
    // Le tour qui obtient enfin le pilote ferme la famine.
    c.note_verrou_pris(238 * MS, 240 * MS);
    let r = c.releve();
    assert_eq!(r.lock_starve_max_us, 140_000, "du premier refus au succes");
    assert_eq!(r.run_to_lock_max_us, 2_000, "le tour qui a reussi, lui, fut court");
    assert_eq!(
        r.responsable(),
        "verrou",
        "une famine de 140 ms ne doit pas se cacher derriere un run_to_lock de 2 ms"
    );
}

#[test]
fn une_seconde_famine_ne_repart_pas_du_premier_refus_de_la_premiere() {
    let c = ChronoHid::neuf();
    c.note_echec_verrou(3, 10 * MS);
    c.note_verrou_pris(0, 20 * MS);
    assert_eq!(c.releve().lock_starve_max_us, 10_000);
    c.note_echec_verrou(3, 100 * MS);
    c.note_verrou_pris(0, 105 * MS);
    let r = c.releve();
    assert_eq!(r.lock_starve_max_us, 10_000, "la premiere reste le maximum");
}

#[test]
fn un_succes_sans_refus_ne_fabrique_pas_de_famine() {
    // Sans le drapeau « pas de famine en cours », chaque tour reussi
    // produirait une famine de la taille de l'horloge.
    let c = ChronoHid::neuf();
    c.note_verrou_pris(1000 * MS, 1001 * MS);
    assert_eq!(c.releve().lock_starve_max_us, 0);
}
