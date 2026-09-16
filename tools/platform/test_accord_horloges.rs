//! Deux horloges qui doivent se confirmer.
//!
//! Le verdict precedent etait `dt != 0 || dm != 0`. Ces tests existent pour
//! qu'aucune des trois situations qu'un OU laissait passer ne repasse : une
//! horloge arretee, deux horloges en desaccord d'ordre de grandeur, et le cas
//! reel releve le 16 septembre 2026.

#[path = "../../src/kernel/time/accord.rs"]
mod accord;

use accord::{accord, depuis_octet, Accord, FENETRE_MINIMALE, TOLERANCE_POURCENT};

// ---------------------------------------------------------------------------
// Les cas que le OU laissait passer
// ---------------------------------------------------------------------------

#[test]
fn une_horloge_arretee_ne_passe_plus() {
    // C'est le pire cas : la moitie du systeme lit un compteur mort, et
    // l'autre moitie avance. Le OU annoncait vert.
    assert_eq!(accord(90, 0), Accord::UneSeuleAvance);
    assert_eq!(accord(0, 90), Accord::UneSeuleAvance);
    assert!(!accord(90, 0).fiable());
    assert!(!accord(0, 90).fiable());
}

#[test]
fn le_releve_fautif_du_16_septembre_est_refuse() {
    // BOUCHAUD_HWPROBE_TIMER progress=1 ticks_delta=23 ms_delta=14
    // Un rapport de 1,64 entre les deux horloges, annonce vert a l'epoque.
    let verdict = accord(23, 14);
    assert_eq!(verdict, Accord::Desaccord);
    assert!(!verdict.fiable());
}

#[test]
fn le_releve_sain_du_meme_jour_est_accepte() {
    // BOUCHAUD_HWPROBE_TIMER progress=1 ticks_delta=90 ms_delta=90
    assert_eq!(accord(90, 90), Accord::Accord);
    assert!(accord(90, 90).fiable());
}

#[test]
fn un_temps_completement_arrete_se_distingue_d_une_horloge_morte() {
    // Les deux cas sont rouges, mais ils ne se corrigent pas de la meme
    // facon : l'un est « rien ne tourne », l'autre « une source est morte ».
    assert_eq!(accord(0, 0), Accord::AucuneNAvance);
    assert_ne!(accord(0, 0), accord(90, 0));
}

// ---------------------------------------------------------------------------
// La tolerance
// ---------------------------------------------------------------------------

#[test]
fn un_ecart_d_un_ordre_de_grandeur_est_toujours_refuse() {
    // Le cas qui compte vraiment : une calibration fausse d'un facteur dix.
    assert_eq!(accord(1000, 100), Accord::Desaccord);
    assert_eq!(accord(100, 1000), Accord::Desaccord);
}

#[test]
fn la_tolerance_est_symetrique() {
    // Inverser les deux compteurs ne doit rien changer au verdict : rapporter
    // l'ecart au plus PETIT rendrait la reponse dependante du sens de
    // l'erreur.
    for (a, b) in [(90u64, 90u64), (23, 14), (100, 60), (1000, 100), (7, 5)] {
        assert_eq!(accord(a, b), accord(b, a), "{} contre {}", a, b);
    }
}

#[test]
fn juste_sous_la_tolerance_passe_et_juste_au_dessus_non() {
    // La frontiere doit etre exactement la constante annoncee, pas une valeur
    // voisine : c'est elle que la ligne de journal publie.
    let petit = 100u64;
    let a_la_limite = petit + TOLERANCE_POURCENT * petit / 100;
    assert_eq!(accord(a_la_limite, petit), Accord::Accord);
    assert_eq!(accord(a_la_limite + 1, petit), Accord::Desaccord);
}

#[test]
fn une_fenetre_trop_courte_ne_conclut_pas_et_ne_rassure_pas() {
    // Sous huit tics, un ecart d'une unite pese plus de douze pour cent : la
    // quantification seule produirait des desaccords. Mais ne pas pouvoir
    // conclure ne doit surtout pas se lire « tout va bien ».
    for (a, b) in [(1u64, 1u64), (1, 2), (2, 1), (7, 7)] {
        let verdict = accord(a, b);
        assert_eq!(verdict, Accord::FenetreTropCourte, "{} contre {}", a, b);
        assert!(!verdict.fiable(), "{} contre {}", a, b);
    }
    // Des huit tics, on conclut.
    assert_eq!(accord(FENETRE_MINIMALE, FENETRE_MINIMALE), Accord::Accord);
}

#[test]
fn des_compteurs_enormes_ne_debordent_pas() {
    // L'ecart est multiplie par cent avant division : un delta proche de
    // u64::MAX ferait deborder une multiplication naive.
    assert_eq!(accord(u64::MAX, u64::MAX), Accord::Accord);
    assert_eq!(accord(u64::MAX, 1), Accord::Desaccord);
}

// ---------------------------------------------------------------------------
// Le verdict traverse un octet
// ---------------------------------------------------------------------------

#[test]
fn le_codage_en_octet_fait_un_aller_retour_exact() {
    for etat in [
        Accord::AucuneNAvance,
        Accord::UneSeuleAvance,
        Accord::Desaccord,
        Accord::FenetreTropCourte,
        Accord::Accord,
    ] {
        assert_eq!(depuis_octet(etat as u8), etat, "{:?}", etat);
    }
}

#[test]
fn un_octet_inconnu_refuse_de_garantir() {
    // Sur une base de temps, le doute penche du cote qui ne garantit rien.
    assert!(!depuis_octet(200).fiable());
}

#[test]
fn seul_l_accord_est_fiable() {
    assert!(!Accord::AucuneNAvance.fiable());
    assert!(!Accord::UneSeuleAvance.fiable());
    assert!(!Accord::Desaccord.fiable());
    assert!(!Accord::FenetreTropCourte.fiable());
    assert!(Accord::Accord.fiable());
}

#[test]
fn chaque_verdict_a_un_nom_distinct() {
    let noms = [
        Accord::AucuneNAvance.nom(),
        Accord::UneSeuleAvance.nom(),
        Accord::Desaccord.nom(),
        Accord::FenetreTropCourte.nom(),
        Accord::Accord.nom(),
    ];
    for (i, a) in noms.iter().enumerate() {
        assert!(!a.is_empty());
        for b in &noms[i + 1..] {
            assert_ne!(a, b);
        }
    }
}
