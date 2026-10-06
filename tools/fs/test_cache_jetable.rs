//! Ce que la persistance abandonne quand `/persist` ne tient plus.
//!
//! BOUCHAUD_PERSIST_CACHE_JETABLE_V1
//!
//! Compile `src/fs/cache_jetable.rs` -- le code meme que le noyau execute -- et
//! verifie :
//!
//!   J1 l'etiquette : seule la signature exacte de la specification compte ;
//!   J2 tout tient -> rien n'est ecarte, meme un cache ;
//!   J3 trop de fichiers (le cas du cache HTTP) -> le cache est ecarte ENTIER,
//!      les temoins et reglages restent ;
//!   J4 trop d'octets -> idem ;
//!   J5 plusieurs caches : le plus gros d'abord, et seulement ce qu'il faut ;
//!   J6 ce qui deborde n'est pas jetable -> NeTientPas, et rien n'est ecarte ;
//!   J7 exhaustif sur de petites configurations : apres un `Ecarte`, le reste
//!      tient TOUJOURS, aucun fichier non jetable n'est jamais ecarte, et un
//!      groupe est ecarte entier ou pas du tout ; `NeTientPas` seulement si
//!      meme sans aucun cache ca ne tient pas.
//!
//! Lance par `tools/ci/run_host_tests.sh`.

#[path = "../../src/fs/cache_jetable.rs"]
mod cache_jetable;

use cache_jetable::*;

const SECTEUR: usize = 512;

fn limites(entrees_max: usize, secteurs_max: u64) -> Limites {
    Limites { entrees_max, secteurs_max, taille_secteur: SECTEUR }
}

fn f(octets: usize, groupe: Option<u32>) -> Fichier {
    Fichier { octets, groupe }
}

#[test]
fn j1_seule_la_signature_exacte_etiquette() {
    assert!(est_etiquette("CACHEDIR.TAG", CONTENU_ETIQUETTE));
    assert!(est_etiquette("CACHEDIR.TAG", SIGNATURE));
    assert!(!est_etiquette("CACHEDIR.TAG", b"Signature: 8a477f597d28d172789f06886806bc5"));
    assert!(!est_etiquette("CACHEDIR.TAG", b""));
    assert!(!est_etiquette("cachedir.tag", CONTENU_ETIQUETTE));
    assert!(!est_etiquette("CACHEDIR.TAG.bak", CONTENU_ETIQUETTE));
    assert!(CONTENU_ETIQUETTE.starts_with(SIGNATURE));
    assert_eq!(SIGNATURE.len(), 43);
}

#[test]
fn j2_tout_tient_rien_n_est_ecarte() {
    let fichiers = [f(100, None), f(4000, Some(0)), f(10, Some(0))];
    let mut e = [true; 1];
    assert_eq!(selectionne(&fichiers, &limites(10, 100), &mut e), Verdict::ToutTient);
    assert_eq!(e, [false]);
}

#[test]
fn j3_trop_de_fichiers_le_cache_part_les_temoins_restent() {
    // Le cas qui a motive le module : 2048 entrees au plus, un cache HTTP de
    // 3000 petites reponses, et la base des cookies a cote.
    let mut fichiers = vec![f(65536, None), f(4096, None)];
    fichiers.extend((0..3000).map(|_| f(900, Some(0))));
    let mut e = [false; 1];
    let v = selectionne(&fichiers, &limites(2048, 131_000), &mut e);
    assert_eq!(v, Verdict::Ecarte { arbres: 1, fichiers: 3000, octets: 3000 * 900 });
    assert_eq!(e, [true]);
}

#[test]
fn j4_trop_d_octets_le_cache_part() {
    let fichiers = [f(1024, None), f(200 * SECTEUR, Some(0)), f(SECTEUR, Some(0))];
    let mut e = [false; 1];
    let v = selectionne(&fichiers, &limites(100, 100), &mut e);
    assert_eq!(v, Verdict::Ecarte { arbres: 1, fichiers: 2, octets: (201 * SECTEUR) as u64 });
    assert_eq!(e, [true]);
}

#[test]
fn j5_le_plus_gros_d_abord_et_seulement_ce_qu_il_faut() {
    // Deux caches : 60 et 30 secteurs ; 10 secteurs non jetables ; borne 50.
    // Ecarter le gros suffit (40 <= 50) : le petit reste.
    let fichiers = [f(10 * SECTEUR, None), f(30 * SECTEUR, Some(1)), f(60 * SECTEUR, Some(0))];
    let mut e = [false; 2];
    let v = selectionne(&fichiers, &limites(100, 50), &mut e);
    assert_eq!(v, Verdict::Ecarte { arbres: 1, fichiers: 1, octets: (60 * SECTEUR) as u64 });
    assert_eq!(e, [true, false]);

    // Borne 30 : il faut les deux.
    let v = selectionne(&fichiers, &limites(100, 30), &mut e);
    assert_eq!(v, Verdict::Ecarte { arbres: 2, fichiers: 2, octets: (90 * SECTEUR) as u64 });
    assert_eq!(e, [true, true]);
}

#[test]
fn j6_ce_qui_deborde_n_est_pas_jetable() {
    let fichiers = [f(60 * SECTEUR, None), f(10 * SECTEUR, Some(0))];
    let mut e = [false; 1];
    assert_eq!(selectionne(&fichiers, &limites(100, 50), &mut e), Verdict::NeTientPas);
    // Rien n'est ecarte : l'echec doit porter sur l'etat complet, pas sur un
    // etat ampute dont on aurait deja jete le cache pour rien.
    assert_eq!(e, [false]);
    let fichiers: Vec<Fichier> = (0..5).map(|_| f(1, None)).collect();
    assert_eq!(selectionne(&fichiers, &limites(4, 100), &mut e), Verdict::NeTientPas);
}

#[test]
fn j6b_un_fichier_vide_ou_un_groupe_vide_ne_trompent_pas() {
    // Un numero de groupe sans fichier ne compte pas comme ecarte.
    let fichiers = [f(SECTEUR, None), f(3 * SECTEUR, Some(2))];
    let mut e = [false; 3];
    let v = selectionne(&fichiers, &limites(10, 2), &mut e);
    assert_eq!(v, Verdict::Ecarte { arbres: 1, fichiers: 1, octets: (3 * SECTEUR) as u64 });
    assert_eq!(e, [false, false, true]);
}

#[test]
fn j7_exhaustif() {
    // Jusqu'a 5 fichiers, tailles 0..=3 secteurs, groupes None/0/1, bornes
    // d'entrees 0..=5 et de secteurs 0..=8.
    let tailles = [0usize, 1, SECTEUR, 2 * SECTEUR + 1];
    let groupes = [None, Some(0u32), Some(1)];
    let mut cas = 0u64;
    for n in 0..=5usize {
        let combinaisons = (tailles.len() * groupes.len()).pow(n as u32);
        for code in 0..combinaisons {
            let mut c = code;
            let mut fichiers = Vec::with_capacity(n);
            for _ in 0..n {
                let k = c % (tailles.len() * groupes.len());
                c /= tailles.len() * groupes.len();
                fichiers.push(f(tailles[k % tailles.len()], groupes[k / tailles.len()]));
            }
            for entrees_max in 0..=5 {
                for secteurs_max in 0..=8u64 {
                    cas += 1;
                    verifie_invariants(&fichiers, &limites(entrees_max, secteurs_max));
                }
            }
        }
    }
    assert!(cas > 1_000_000, "{cas} cas seulement");
}

fn tient(fichiers: &[Fichier], l: &Limites, garde: impl Fn(&Fichier) -> bool) -> bool {
    let restants: Vec<&Fichier> = fichiers.iter().filter(|x| garde(x)).collect();
    let s: u64 = restants.iter().map(|x| secteurs(x.octets, l.taille_secteur)).sum();
    restants.len() <= l.entrees_max && s <= l.secteurs_max
}

fn verifie_invariants(fichiers: &[Fichier], l: &Limites) {
    let mut e = [false; 2];
    let v = selectionne(fichiers, l, &mut e);
    let tout = tient(fichiers, l, |_| true);
    let sans_caches = tient(fichiers, l, |x| x.groupe.is_none());
    match v {
        Verdict::ToutTient => {
            assert!(tout, "{fichiers:?} {l:?}");
            assert_eq!(e, [false, false]);
        }
        Verdict::NeTientPas => {
            assert!(!tout && !sans_caches, "{fichiers:?} {l:?}");
            assert_eq!(e, [false, false]);
        }
        Verdict::Ecarte { arbres, fichiers: n, octets } => {
            assert!(!tout, "rien a ecarter : {fichiers:?} {l:?}");
            // Le reste tient.
            assert!(tient(fichiers, l, |x| !x.groupe.map_or(false, |g| e[g as usize])), "{fichiers:?} {l:?} {e:?}");
            // Seuls des groupes PRESENTS sont ecartes, et les comptes sont justes.
            let ecartes: Vec<&Fichier> = fichiers.iter().filter(|x| x.groupe.map_or(false, |g| e[g as usize])).collect();
            assert_eq!(ecartes.len(), n);
            assert_eq!(ecartes.iter().map(|x| x.octets as u64).sum::<u64>(), octets);
            let presents = (0..2u32).filter(|g| e[*g as usize]).count() as u32;
            assert_eq!(presents, arbres);
            for g in 0..2u32 {
                if e[g as usize] {
                    assert!(fichiers.iter().any(|x| x.groupe == Some(g)));
                }
            }
            // Minimalite locale : si un seul groupe a ete ecarte, aucun
            // n'aurait suffi.
            if arbres == 1 {
                assert!(!tout);
            }
        }
    }
}
