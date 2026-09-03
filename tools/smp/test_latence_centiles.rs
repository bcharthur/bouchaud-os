//! Preuve hote des centiles de latence reveil -> coeur.
//!
//! Le module de production est inclus tel quel : ce qui est verifie ici est
//! l'arithmetique qui produira `[SCHED-NG-CENTILES]` dans les traces.
//!
//! # Pourquoi des centiles, et pas une moyenne
//!
//! Une moyenne noie les quelques pour cent de reveils lents qui font qu'une
//! interface « accroche ». Un p99 a 30 ms, c'est un clic sur cent qui repond
//! en retard : visible a l'ecran, invisible dans `avg_ns`.
//!
//! # Le sens de l'erreur
//!
//! Une classe de l'histogramme couvre un facteur deux. Un centile lu est donc
//! une BORNE SUPERIEURE de la vraie valeur, jamais une sous-estimation -- le
//! bon sens de l'erreur quand on s'en sert comme budget.

/// Le module de production journalise sur le port serie. Sur l'hote, la trace
/// n'a pas de destination : la macro est reduite a la verification du format,
/// ce qui garde l'appel type sans avoir besoin du noyau.
#[macro_export]
macro_rules! serial_println {
    ($($arg:tt)*) => {{ let _ = format!($($arg)*); }};
}

#[path = "../../src/kernel/scheduler/latency.rs"]
mod latency;

use latency::{borne_superieure, centile_depuis, classe, CLASSES};

fn histogramme(echantillons: &[u64]) -> ([u64; CLASSES], u64) {
    let mut h = [0u64; CLASSES];
    for &ns in echantillons {
        h[classe(ns)] += 1;
    }
    (h, echantillons.len() as u64)
}

#[test]
fn la_classe_est_la_puissance_de_deux() {
    assert_eq!(classe(0), 0);
    assert_eq!(classe(1), 1);
    assert_eq!(classe(2), 2);
    assert_eq!(classe(3), 2);
    assert_eq!(classe(4), 3);
    assert_eq!(classe(u64::MAX), CLASSES - 1, "rien ne deborde de l'histogramme");
}

#[test]
fn la_borne_superieure_majore_toujours_l_echantillon() {
    for ns in [0u64, 1, 2, 3, 999, 1_000, 100_000, 16_000_000, 1_000_000_000] {
        assert!(
            borne_superieure(classe(ns)) >= ns,
            "la classe de {ns} ns doit le majorer, pas le sous-estimer"
        );
    }
}

/// Quatre-vingt-dix-huit echantillons a 1 us et deux a 100 ms : la moyenne
/// reste basse, le p99 doit voir les retardataires. C'est exactement le cas que
/// `avg_ns` cachait.
#[test]
fn le_p99_voit_ce_que_la_moyenne_noie() {
    let mut echantillons = vec![1_000u64; 98];
    echantillons.push(100_000_000);
    echantillons.push(100_000_000);
    let (h, total) = histogramme(&echantillons);

    let moyenne: u64 = echantillons.iter().sum::<u64>() / total;
    assert!(moyenne < 4_000_000, "la moyenne reste basse : {moyenne} ns");

    let p50 = centile_depuis(&h, total, 50);
    let p99 = centile_depuis(&h, total, 99);
    assert!(p50 <= 2_048, "le p50 doit rester sur les rapides : {p50} ns");
    assert!(
        p99 >= 100_000_000,
        "le p99 doit majorer le retardataire : {p99} ns"
    );
}

#[test]
fn les_centiles_sont_monotones() {
    let echantillons: Vec<u64> = (1..=1000u64).map(|i| i * 1_000).collect();
    let (h, total) = histogramme(&echantillons);
    let p50 = centile_depuis(&h, total, 50);
    let p95 = centile_depuis(&h, total, 95);
    let p99 = centile_depuis(&h, total, 99);
    assert!(p50 <= p95, "p50={p50} p95={p95}");
    assert!(p95 <= p99, "p95={p95} p99={p99}");
}

#[test]
fn un_echantillon_unique_est_son_propre_centile() {
    let (h, total) = histogramme(&[42_000]);
    for centile in [50u64, 95, 99] {
        let valeur = centile_depuis(&h, total, centile);
        assert!(valeur >= 42_000, "p{centile}={valeur} doit majorer 42000");
        assert!(valeur <= 65_536, "p{centile}={valeur} ne doit pas majorer grossierement");
    }
}

/// Le rang est un PLAFOND (methode du rang le plus proche).
///
/// Sur dix echantillons dont un seul est lent, un rang PLANCHER placerait le
/// p95 sur le neuvieme -- donc sur un rapide --, et un budget p95 resterait
/// vert en contenant le retardataire. Le plafond le fait tomber sur le
/// dixieme.
#[test]
fn le_rang_est_un_plafond() {
    let mut echantillons = vec![1_000u64; 9];
    echantillons.push(1_000_000_000);
    let (h, total) = histogramme(&echantillons);

    let plancher = (total * 95 / 100) as usize;
    assert_eq!(plancher, 9, "le rang plancher designerait un echantillon rapide");

    let p95 = centile_depuis(&h, total, 95);
    assert!(
        p95 >= 1_000_000_000,
        "le p95 doit majorer le retardataire, pas le contenir : {p95} ns"
    );
}

/// Une classe d'ordonnancement inconnue ne doit rien inventer.
///
/// Le releve boucle sur les classes connues, mais `centiles` est publique :
/// un indice hors bornes doit rendre un releve VIDE, pas lire a cote du
/// tableau ni fabriquer une valeur.
#[test]
fn une_classe_inconnue_ne_rend_rien() {
    let c = latency::centiles(99);
    assert_eq!(c.count, 0);
    assert_eq!(c.p50_ns, 0);
    assert_eq!(c.p99_ns, 0);
    assert_eq!(c.max_ns, 0);
}

/// Les deux classes d'ordonnancement sont comptees SEPAREMENT.
///
/// Melangees, la classe interactive -- minoritaire en nombre d'evenements --
/// disparaissait dans la masse du travail de fond, et c'est precisement elle
/// que le chantier 2 cherche a ameliorer.
#[test]
fn interactive_et_normale_ne_se_melangent_pas() {
    for _ in 0..100 {
        latency::record(1_000, true);
    }
    for _ in 0..100 {
        latency::record(50_000_000, false);
    }

    let interactive = latency::centiles(latency::INTERACTIVE);
    let normale = latency::centiles(latency::NORMALE);

    assert_eq!(interactive.count, 100);
    assert_eq!(normale.count, 100);
    assert!(
        interactive.p99_ns < normale.p50_ns,
        "les histogrammes doivent etre disjoints : interactive p99={} normale p50={}",
        interactive.p99_ns, normale.p50_ns
    );
    assert!(interactive.max_ns >= 1_000);
    assert!(normale.max_ns >= 50_000_000);
}
