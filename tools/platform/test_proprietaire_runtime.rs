//! Qui tient le pilote xHCI, depuis quand, et combien de temps au pire.
//!
//! Ce que ces epreuves defendent : le critere d'acceptation C -- le verrou
//! revient TOUJOURS a `Aucun` --, et un releve capable de nommer le
//! responsable d'une tenue trop longue au lieu de constater qu'il y en a eu
//! une.

#[path = "../../src/drivers/usb/proprietaire_runtime.rs"]
mod pr;

use pr::{Proprietaire, Registre};

const MS: u64 = 1_000_000;

#[test]
fn un_registre_neuf_est_libre() {
    let r = Registre::neuf();
    let e = r.etat(0);
    assert!(e.libre());
    assert_eq!(e.proprietaire, Proprietaire::Aucun);
    assert_eq!(e.tenue_courante_ns, 0);
    assert_eq!(e.prises, 0);
}

#[test]
fn la_prise_et_la_liberation_ramenent_a_aucun() {
    let r = Registre::neuf();
    r.note_prise(Proprietaire::Hid, 10 * MS);
    let pendant = r.etat(13 * MS);
    assert_eq!(pendant.proprietaire, Proprietaire::Hid);
    assert_eq!(pendant.tenue_courante_ns, 3 * MS);
    assert!(!pendant.libre());

    assert_eq!(r.note_liberation(15 * MS), 5 * MS);
    let apres = r.etat(20 * MS);
    assert!(apres.libre(), "critere C : le verrou revient a Aucun");
    assert_eq!(apres.tenue_courante_ns, 0, "libre ne tient depuis rien");
    assert_eq!(apres.prises, 1);
    assert_eq!(apres.derniere_liberation_ns, 15 * MS);
}

#[test]
fn le_pire_et_son_auteur_voyagent_ensemble() {
    // « La pire tenue fait 480 ms » ne se corrige pas. « ... et c'est le
    // systeme de fichiers » se corrige.
    let r = Registre::neuf();
    r.note_prise(Proprietaire::Hid, 0);
    r.note_liberation(2 * MS);
    r.note_prise(Proprietaire::SystemeDeFichiers, 10 * MS);
    r.note_liberation(490 * MS);
    r.note_prise(Proprietaire::VidageBlackbox, 600 * MS);
    r.note_liberation(601 * MS);

    let e = r.etat(700 * MS);
    assert_eq!(e.tenue_max_ns, 480 * MS);
    assert_eq!(e.tenue_max_proprietaire, Proprietaire::SystemeDeFichiers);
    assert_eq!(e.prises, 3);
    assert!(e.libre());
}

#[test]
fn le_pire_ne_recule_jamais() {
    let r = Registre::neuf();
    r.note_prise(Proprietaire::Enumeration, 0);
    r.note_liberation(300 * MS);
    r.note_prise(Proprietaire::Hid, 400 * MS);
    r.note_liberation(401 * MS);
    let e = r.etat(500 * MS);
    assert_eq!(e.tenue_max_ns, 300 * MS);
    assert_eq!(e.tenue_max_proprietaire, Proprietaire::Enumeration);
}

#[test]
fn les_contentions_et_les_expirations_se_comptent_a_part() {
    // Renoncer tout de suite et abandonner apres une attente bornee sont deux
    // evenements differents : le premier est normal mille fois par seconde,
    // le second dit que le verrou a ete tenu plus longtemps qu'un budget.
    let r = Registre::neuf();
    r.note_contention();
    r.note_contention();
    r.note_expiration();
    let e = r.etat(0);
    assert_eq!(e.contentions, 2);
    assert_eq!(e.expirations, 1);
}

#[test]
fn une_tenue_trop_longue_se_detecte_pendant_qu_elle_dure() {
    // Le point : la detecter APRES coup ne sert a rien, le clavier a deja
    // saute ses tours. Le seuil est un argument, parce que la reponse depend
    // du chemin et non du module.
    let r = Registre::neuf();
    assert!(
        !r.detenu_trop_longtemps(0, 50 * MS),
        "un verrou libre n'est jamais tenu trop longtemps"
    );
    r.note_prise(Proprietaire::SystemeDeFichiers, 100 * MS);
    assert!(!r.detenu_trop_longtemps(120 * MS, 50 * MS));
    assert!(r.detenu_trop_longtemps(151 * MS, 50 * MS));
    r.note_liberation(160 * MS);
    assert!(!r.detenu_trop_longtemps(1_000 * MS, 50 * MS));
}

#[test]
fn chaque_proprietaire_a_un_code_et_un_nom_stables() {
    // Le code voyage dans l'archive ; le nom dans la console. Les deux
    // doivent designer la meme chose, sinon un releve lu sur la machine et
    // une archive lue sur l'hote se contredisent.
    for qui in [
        Proprietaire::Aucun,
        Proprietaire::Hid,
        Proprietaire::ReplieHid,
        Proprietaire::SystemeDeFichiers,
        Proprietaire::VidageBlackbox,
        Proprietaire::Enumeration,
        Proprietaire::Diagnostic,
    ] {
        assert_eq!(Proprietaire::depuis_code(qui.code()), qui);
        assert!(!qui.nom().is_empty());
    }
    assert_eq!(Proprietaire::depuis_code(200), Proprietaire::Aucun);
}

#[test]
fn une_horloge_qui_recule_ne_fabrique_pas_une_tenue_geante() {
    // `monotonic_ns` peut rendre deux fois la meme valeur, et un compteur
    // recalibre peut reculer d'un cran. Une soustraction naive rendrait alors
    // une tenue de dix-huit milliards d'annees, qui deviendrait le pire de la
    // session pour toujours.
    let r = Registre::neuf();
    r.note_prise(Proprietaire::Hid, 500 * MS);
    assert_eq!(r.note_liberation(499 * MS), 0);
    assert_eq!(r.etat(600 * MS).tenue_max_ns, 0);
}
