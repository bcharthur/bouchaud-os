//! Preuve hote des regles de securite de l'ABI native Bouchaud.
//!
//! Les deux modules de production sont inclus tels quels :
//! `src/kernel/native/abi/types.rs` (l'algebre des droits) et
//! `src/kernel/native/handle/politique.rs` (les decisions). Ce qui est mis a
//! l'epreuve ici est le code qui decide, dans le noyau, si un processus peut
//! toucher a un objet.
//!
//! # Pourquoi les cas NEGATIFS sont le sujet
//!
//! Une isolation ne se prouve pas en montrant que ce qui est permis marche :
//! cela, un systeme sans aucune verification le fait aussi. Elle se prouve en
//! montrant que ce qui est INTERDIT est refuse, et refuse avec la BONNE erreur
//! -- `BadHandle` quand le handle n'existe pas, `AccessDenied` quand il existe
//! mais ne porte pas le droit, `WrongType` quand il porte le droit mais designe
//! autre chose. Les confondre renseigne un appelant hostile sur ce qu'il n'a
//! pas le droit de savoir.
//!
//! # Le defaut que ces tests ont attrape
//!
//! `HandleTable::export` portait :
//!
//!     entry.rights = entry.rights.intersection(entry.rights);
//!
//! sous un commentaire promettant qu'aucun droit n'est gagne par IPC.
//! L'intersection d'un ensemble avec lui-meme est cet ensemble : la ligne ne
//! faisait rien, et surtout il n'existait AUCUN moyen d'attenuer. Un courtier
//! detenant une region en lecture-ecriture ne pouvait pas en donner une vue en
//! lecture seule -- il donnait tout, ou rien.

#[path = "../../src/kernel/native/abi/types.rs"]
mod types;

mod politique {
    use super::types::{Error, ObjectKind, Result, Rights};
    include!("../../src/kernel/native/handle/politique_corps.rs");
}

use politique::*;
use types::{Error, ObjectKind, Rights};

// ---------------------------------------------------------------------------
// L'algebre des droits.
// ---------------------------------------------------------------------------

#[test]
fn tous_contient_chaque_droit_defini() {
    for droit in [
        Rights::READ, Rights::WRITE, Rights::SIGNAL, Rights::MAP,
        Rights::DUP, Rights::TRANSFER, Rights::INSPECT, Rights::WAIT,
    ] {
        assert!(
            Rights::TOUS.contains(droit),
            "un droit hors de TOUS ne pourrait jamais etre transfere sans \
             attenuation involontaire : {droit:?}"
        );
    }
}

#[test]
fn aucune_regle_n_ajoute_de_droit() {
    // L'invariant central : toute derivation est bornee par sa source.
    let source = Rights::READ | Rights::DUP | Rights::TRANSFER;
    let demande_trop = Rights::TOUS;

    assert_eq!(verifie_duplication(source, demande_trop), Err(Error::AccessDenied));
    let transfere = verifie_transfert(source, demande_trop).unwrap();
    assert!(borne_par(transfere, source), "{transfere:?} depasse {source:?}");
    assert_eq!(transfere, source, "sans attenuation, le transfert conserve exactement");
}

// ---------------------------------------------------------------------------
// Droits insuffisants.
// ---------------------------------------------------------------------------

#[test]
fn un_droit_manquant_donne_acces_refuse() {
    let lecture_seule = Rights::READ | Rights::INSPECT;
    assert_eq!(verifie_acces(lecture_seule, Rights::READ), Ok(()));
    assert_eq!(verifie_acces(lecture_seule, Rights::WRITE), Err(Error::AccessDenied));
    assert_eq!(
        verifie_acces(lecture_seule, Rights::READ | Rights::WRITE),
        Err(Error::AccessDenied),
        "un sous-ensemble partiel ne suffit pas : tous les droits demandes sont exiges"
    );
}

#[test]
fn aucun_droit_ne_permet_rien_sauf_ne_rien_demander() {
    assert_eq!(verifie_acces(Rights::NONE, Rights::NONE), Ok(()));
    for droit in [Rights::READ, Rights::WRITE, Rights::MAP, Rights::SIGNAL] {
        assert_eq!(verifie_acces(Rights::NONE, droit), Err(Error::AccessDenied));
    }
}

// ---------------------------------------------------------------------------
// Mauvais genre d'objet.
// ---------------------------------------------------------------------------

#[test]
fn un_mauvais_genre_donne_wrong_type_et_non_acces_refuse() {
    assert_eq!(verifie_genre(ObjectKind::Channel, ObjectKind::Channel), Ok(()));
    assert_eq!(
        verifie_genre(ObjectKind::Event, ObjectKind::Channel),
        Err(Error::WrongType),
        "un handle valide vers le mauvais objet est une erreur de TYPE : dire \
         `AccessDenied` laisserait croire qu'un droit de plus suffirait"
    );
    assert_eq!(
        verifie_genre(ObjectKind::SharedRegion, ObjectKind::WaitSet),
        Err(Error::WrongType)
    );
    assert_eq!(
        verifie_genre(ObjectKind::LegacyFile, ObjectKind::SharedRegion),
        Err(Error::WrongType)
    );
}

// ---------------------------------------------------------------------------
// Generation perimee : le probleme ABA des handles.
// ---------------------------------------------------------------------------

#[test]
fn une_generation_perimee_donne_bad_handle() {
    assert_eq!(verifie_generation(7, 7), Ok(()));
    assert_eq!(
        verifie_generation(8, 7),
        Err(Error::BadHandle),
        "l'emplacement a ete recycle : le vieux handle designe l'objet SUIVANT"
    );
    assert_eq!(verifie_generation(7, 8), Err(Error::BadHandle));
}

#[test]
fn une_generation_nulle_n_est_jamais_valide() {
    // Zero signifie « jamais occupe » : un handle qui la porte ne peut venir
    // que d'un champ non initialise ou d'une valeur fabriquee.
    assert_eq!(verifie_generation(0, 0), Err(Error::BadHandle));
    assert_eq!(verifie_generation(0, 1), Err(Error::BadHandle));
    assert_eq!(verifie_generation(1, 0), Err(Error::BadHandle));
}

/// L'identite d'un handle survit a un aller-retour, et ne fuit pas de bits.
#[test]
fn l_identite_d_un_handle_est_stable_et_positive() {
    use types::HandleId;
    let id = HandleId::new(42, 7);
    assert_eq!(id.slot(), 42);
    assert_eq!(id.generation(), 7);
    assert!(id.valid());
    assert_eq!(HandleId::from_raw(id.raw()), id);
    assert!(
        id.raw() <= i64::MAX as u64,
        "un handle doit pouvoir revenir en `i64` positif : les erreurs natives \
         sont negatives"
    );

    // Le bit 63 d'une valeur fabriquee est efface, jamais interprete.
    let hostile = HandleId::from_raw(u64::MAX);
    assert!(hostile.raw() <= i64::MAX as u64);

    assert!(!HandleId::INVALID.valid());
    assert!(!HandleId::new(1, 0).valid(), "generation nulle = handle invalide");
}

// ---------------------------------------------------------------------------
// Duplication : attenuation, jamais elevation.
// ---------------------------------------------------------------------------

#[test]
fn dupliquer_sans_le_droit_dup_est_refuse() {
    let sans_dup = Rights::READ | Rights::WRITE | Rights::TRANSFER;
    assert_eq!(
        verifie_duplication(sans_dup, Rights::READ),
        Err(Error::AccessDenied),
        "un handle donne sans DUP ne doit pas pouvoir se multiplier : c'est ce \
         qui borne la diffusion d'une capacite"
    );
}

#[test]
fn dupliquer_peut_attenuer() {
    let source = Rights::READ | Rights::WRITE | Rights::DUP | Rights::INSPECT;
    let lecture = verifie_duplication(source, Rights::READ).unwrap();
    assert_eq!(lecture, Rights::READ);
    assert!(borne_par(lecture, source));
    // Et le duplicata attenue ne peut plus, lui, se dupliquer.
    assert_eq!(verifie_duplication(lecture, Rights::READ), Err(Error::AccessDenied));
}

#[test]
fn dupliquer_ne_peut_pas_elever() {
    let source = Rights::READ | Rights::DUP;
    assert_eq!(
        verifie_duplication(source, Rights::READ | Rights::WRITE),
        Err(Error::AccessDenied),
        "demander plus que la source doit etre REFUSE, pas silencieusement rogne : \
         rogner laisserait l'appelant croire qu'il detient ce qu'il a demande"
    );
}

#[test]
fn dupliquer_a_l_identique_est_permis() {
    let source = Rights::READ | Rights::WRITE | Rights::DUP;
    assert_eq!(verifie_duplication(source, source), Ok(source));
}

// ---------------------------------------------------------------------------
// Transfert : le cas que le code ne savait pas faire.
// ---------------------------------------------------------------------------

#[test]
fn transferer_sans_le_droit_transfer_est_refuse() {
    let local = Rights::READ | Rights::WRITE | Rights::DUP;
    assert_eq!(
        verifie_transfert(local, Rights::TOUS),
        Err(Error::AccessDenied),
        "sans TRANSFER, la capacite ne franchit pas la frontiere du processus"
    );
}

/// LE CAS QUI N'EXISTAIT PAS : donner une vue en lecture seule d'une region
/// qu'on detient en lecture-ecriture.
#[test]
fn un_courtier_peut_donner_une_vue_en_lecture_seule() {
    let courtier = Rights::READ | Rights::WRITE | Rights::MAP
        | Rights::DUP | Rights::TRANSFER | Rights::INSPECT;

    let vue = verifie_transfert(courtier, Rights::READ | Rights::MAP).unwrap();
    assert_eq!(vue, Rights::READ | Rights::MAP);
    assert!(!vue.contains(Rights::WRITE), "le moteur de rendu ne doit pas ecrire");
    assert!(!vue.contains(Rights::TRANSFER), "ni repasser la capacite plus loin");
    assert!(!vue.contains(Rights::DUP), "ni la multiplier");
    assert!(borne_par(vue, courtier));
}

#[test]
fn le_masque_de_transfert_est_un_plafond_pas_une_commande() {
    // Contrairement a la duplication, demander plus n'est pas une faute : cela
    // veut dire « tout ce que je peux donner ». C'est ce qui rend
    // `Rights::TOUS` utilisable comme « ne rien attenuer ».
    let source = Rights::READ | Rights::TRANSFER;
    assert_eq!(verifie_transfert(source, Rights::TOUS), Ok(source));
    assert_eq!(
        verifie_transfert(source, Rights::READ | Rights::WRITE | Rights::TRANSFER),
        Ok(source),
        "le masque ne peut pas ajouter WRITE"
    );
}

#[test]
fn un_transfert_peut_retirer_le_droit_de_retransferer() {
    let source = Rights::READ | Rights::TRANSFER;
    let feuille = verifie_transfert(source, Rights::READ).unwrap();
    assert_eq!(feuille, Rights::READ);
    assert_eq!(
        verifie_transfert(feuille, Rights::TOUS),
        Err(Error::AccessDenied),
        "une capacite feuille ne doit pas pouvoir continuer sa route"
    );
}

/// La chaine de delegation ne peut que RETRECIR, quelle que soit sa longueur.
#[test]
fn une_chaine_de_delegation_ne_grandit_jamais() {
    let racine = Rights::TOUS;
    let mut courant = racine;
    let masques = [
        Rights::READ | Rights::WRITE | Rights::DUP | Rights::TRANSFER,
        Rights::READ | Rights::TRANSFER | Rights::DUP,
        Rights::READ | Rights::TRANSFER,
        Rights::READ,
    ];
    for masque in masques {
        courant = verifie_transfert(courant, masque).unwrap();
        assert!(borne_par(courant, racine), "{courant:?} depasse la racine");
    }
    assert_eq!(courant, Rights::READ);

    // Et une tentative de remonter echoue.
    assert_eq!(verifie_transfert(courant, Rights::TOUS), Err(Error::AccessDenied));
}

/// Un transfert croise -- duplication attenuee puis transfert attenue -- reste
/// borne par la racine. C'est le motif exact d'un courtier qui prepare une
/// capacite avant de l'envoyer.
#[test]
fn duplication_puis_transfert_restent_bornes() {
    let racine = Rights::READ | Rights::WRITE | Rights::MAP
        | Rights::DUP | Rights::TRANSFER | Rights::INSPECT;

    // Le courtier prepare une copie attenuee...
    let copie = verifie_duplication(racine, Rights::READ | Rights::MAP | Rights::TRANSFER).unwrap();
    // ... puis la transfere en retirant encore le droit de retransferer.
    let livree = verifie_transfert(copie, Rights::READ | Rights::MAP).unwrap();

    assert_eq!(livree, Rights::READ | Rights::MAP);
    assert!(borne_par(livree, copie));
    assert!(borne_par(copie, racine));
    assert!(!livree.contains(Rights::WRITE));
}

// ---------------------------------------------------------------------------
// Les droits par defaut ne doivent pas etre plus larges que TOUS.
// ---------------------------------------------------------------------------

#[test]
fn les_droits_par_defaut_sont_bornes_et_coherents() {
    for (nom, defaut) in [
        ("channel", Rights::CHANNEL_DEFAULT),
        ("event", Rights::EVENT_DEFAULT),
        ("waitset", Rights::WAITSET_DEFAULT),
        ("shm", Rights::SHM_DEFAULT),
    ] {
        assert!(borne_par(defaut, Rights::TOUS), "{nom} sort de TOUS");
        assert!(
            defaut.contains(Rights::INSPECT),
            "{nom} : un objet qu'on ne peut pas inspecter ne peut pas etre \
             diagnostique"
        );
    }
    // Une region partagee ne s'attend pas, elle se lit : pas de WAIT.
    assert!(!Rights::SHM_DEFAULT.contains(Rights::WAIT));
    // Un evenement se signale, il ne se lit pas comme un flux.
    assert!(!Rights::EVENT_DEFAULT.contains(Rights::READ));
}
