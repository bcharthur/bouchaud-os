//! LE PROFIL DE DEMARRAGE, A L'HOTE.
//!
//! # Ce que ce banc protege
//!
//! Le demarrage du navigateur ne s'observe qu'au bout d'une construction
//! complete de Ladybird, sur une machine physique, une fois. Les erreurs que
//! ce calcul peut commettre ne font echouer aucun test d'integration : elles
//! rendent seulement un tableau qui designe la mauvaise etape, et l'on
//! optimise pendant des jours le processus qui n'y etait pour rien.
//!
//! Les proprietes verifiees ici sont celles dont depend cette designation :
//!
//!   * un jalon manquant vaut `None` et jamais zero -- une colonne a zero se
//!     lit « cette etape est gratuite » ;
//!   * un jalon manquant ne casse pas les ecarts suivants : ils se mesurent
//!     depuis le dernier jalon CONNU ;
//!   * le premier instant l'emporte -- un second WebContent ne repousse pas
//!     le jalon « rendu » ;
//!   * la somme des ecarts fait le total, sans quoi le tableau n'additionne
//!     pas a ce qu'il annonce ;
//!   * une horloge qui recule ne cree pas une etape gigantesque ;
//!   * l'etape la plus longue est bien celle qui dure le plus longtemps.

#[path = "../../src/kernel/navigateur/demarrage.rs"]
mod demarrage;

use demarrage::{Jalon, Profil, JALONS};

const MS: u64 = 1_000_000;

/// Un demarrage complet et ordinaire, pour les cas qui n'etudient pas un trou.
fn demarrage_complet() -> Profil {
    let mut profil = Profil::neuf();
    profil.note(Jalon::Clic, 0);
    profil.note(Jalon::Courtier, 100 * MS);
    profil.note(Jalon::Reseau, 400 * MS);
    profil.note(Jalon::Decodeur, 450 * MS);
    profil.note(Jalon::Composition, 500 * MS);
    profil.note(Jalon::Rendu, 900 * MS);
    profil.note(Jalon::PremiereTrame, 1_000 * MS);
    profil
}

#[test]
fn un_jalon_manquant_vaut_rien_et_pas_zero() {
    let mut profil = Profil::neuf();
    profil.note(Jalon::Clic, 0);
    profil.note(Jalon::Courtier, 100 * MS);

    // Le Compositor n'a jamais demarre.
    assert_eq!(profil.segment_ns(Jalon::Composition), None);
    assert!(!profil.vu(Jalon::Composition));
    assert_eq!(profil.instant(Jalon::Composition), None);
    // Et le demarrage n'a pas abouti : pas de premiere trame.
    assert!(!profil.abouti());
    assert_eq!(profil.total_ns(), None);
}

#[test]
fn un_trou_ne_casse_pas_les_ecarts_suivants() {
    // LA REGLE QUI REND LE TABLEAU UTILISABLE QUAND QUELQUE CHOSE MANQUE.
    //
    // Le Compositor ne demarre pas. L'ecart du rendu doit alors se mesurer
    // depuis le DECODEUR -- le dernier jalon connu -- et non depuis un
    // instant qui n'existe pas.
    let mut profil = Profil::neuf();
    profil.note(Jalon::Clic, 0);
    profil.note(Jalon::Courtier, 100 * MS);
    profil.note(Jalon::Reseau, 400 * MS);
    profil.note(Jalon::Decodeur, 450 * MS);
    profil.note(Jalon::Rendu, 900 * MS);
    profil.note(Jalon::PremiereTrame, 1_000 * MS);

    assert_eq!(profil.segment_ns(Jalon::Composition), None);
    assert_eq!(profil.segment_ns(Jalon::Rendu), Some(450 * MS));
    assert_eq!(profil.total_ns(), Some(1_000 * MS));

    let mut manquants = [Jalon::Clic; JALONS];
    let combien = profil.manquants(&mut manquants);
    assert_eq!(combien, 1);
    assert_eq!(manquants[0], Jalon::Composition);
}

#[test]
fn le_premier_instant_l_emporte() {
    // Le portage lance un WebContent par onglet. Le deuxieme ne doit pas
    // repousser le jalon : le demarrage s'arrete a la premiere trame.
    let mut profil = Profil::neuf();
    profil.note(Jalon::Clic, 0);
    assert!(profil.note(Jalon::Rendu, 900 * MS), "le premier pose le jalon");
    assert!(!profil.note(Jalon::Rendu, 5_000 * MS), "le second ne le repousse pas");
    assert_eq!(profil.instant(Jalon::Rendu), Some(900 * MS));
}

#[test]
fn la_somme_des_ecarts_fait_le_total() {
    // Sans cet invariant, le tableau n'additionne pas a ce qu'il annonce --
    // et c'est la premiere chose qu'un lecteur verifie de tete.
    let profil = demarrage_complet();
    let mut somme = 0u64;
    for rang in 1..JALONS {
        if let Some(jalon) = Jalon::depuis_rang(rang) {
            somme += profil.segment_ns(jalon).unwrap_or(0);
        }
    }
    assert_eq!(Some(somme), profil.total_ns());
}

#[test]
fn la_somme_fait_le_total_meme_avec_un_trou() {
    let mut profil = Profil::neuf();
    profil.note(Jalon::Clic, 0);
    profil.note(Jalon::Courtier, 100 * MS);
    profil.note(Jalon::Rendu, 900 * MS);
    profil.note(Jalon::PremiereTrame, 1_000 * MS);

    let mut somme = 0u64;
    for rang in 1..JALONS {
        if let Some(jalon) = Jalon::depuis_rang(rang) {
            somme += profil.segment_ns(jalon).unwrap_or(0);
        }
    }
    assert_eq!(Some(somme), profil.total_ns());
}

#[test]
fn une_horloge_qui_recule_ne_cree_pas_une_etape_geante() {
    // Entre deux coeurs, l'horloge peut reculer de quelques nanosecondes. En
    // arithmetique non signee, un ecart negatif devient un nombre gigantesque
    // -- et ce nombre-la designerait l'etape a optimiser.
    let mut profil = Profil::neuf();
    profil.note(Jalon::Clic, 1_000);
    profil.note(Jalon::Courtier, 500);
    assert_eq!(profil.segment_ns(Jalon::Courtier), Some(0));
    profil.note(Jalon::PremiereTrame, 900);
    assert_eq!(profil.total_ns(), Some(0));
}

#[test]
fn l_etape_la_plus_longue_est_nommee() {
    let profil = demarrage_complet();
    let (jalon, duree) = profil.plus_long().expect("un demarrage complet a des etapes");
    // Les six ecarts du demarrage complet, en millisecondes :
    //   courtier 100, reseau 300, decodeur 50, composition 50,
    //   rendu 400, premiere_trame 100.
    // Le rendu domine, et c'est lui qu'il faut nommer.
    assert_eq!(jalon, Jalon::Rendu);
    assert_eq!(duree, 400 * MS);
}

#[test]
fn un_profil_vierge_ne_designe_rien() {
    let profil = Profil::neuf();
    assert!(profil.plus_long().is_none(), "None dit « rien mesure », pas « rien a optimiser »");
    assert!(!profil.abouti());
    assert_eq!(profil.total_ns(), None);

    let mut manquants = [Jalon::Clic; JALONS];
    assert_eq!(profil.manquants(&mut manquants), JALONS);
}

#[test]
fn un_nouveau_demarrage_efface_le_precedent() {
    let mut profil = demarrage_complet();
    profil.reinitialise();
    assert!(!profil.abouti());
    assert_eq!(profil.instant(Jalon::Rendu), None);
    // Et le jalon peut etre repose : sans cela, la seconde ouverture du
    // navigateur ne serait jamais profilee.
    assert!(profil.note(Jalon::Rendu, 42));
}

#[test]
fn chaque_jalon_a_un_rang_unique_et_reversible() {
    // Le rang sert d'indice de tableau. Deux jalons au meme rang
    // s'ecraseraient silencieusement.
    let tous = [
        Jalon::Clic,
        Jalon::Courtier,
        Jalon::Reseau,
        Jalon::Decodeur,
        Jalon::Composition,
        Jalon::Rendu,
        Jalon::PremiereTrame,
    ];
    assert_eq!(tous.len(), JALONS);
    for (index, jalon) in tous.iter().enumerate() {
        assert_eq!(jalon.rang(), index);
        assert_eq!(Jalon::depuis_rang(index), Some(*jalon));
        assert!(!jalon.nom().is_empty());
    }
    assert_eq!(Jalon::depuis_rang(JALONS), None);
}

#[test]
fn un_tampon_court_ne_deborde_pas() {
    // Le noyau appelle `manquants` depuis un chemin de diagnostic borne.
    let profil = Profil::neuf();
    let mut petit = [Jalon::Clic; 2];
    assert_eq!(profil.manquants(&mut petit), 2);
}
