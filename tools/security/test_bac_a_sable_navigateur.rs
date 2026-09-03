//! Preuve hote du bac a sable des roles du navigateur.
//!
//! Les modules de production sont inclus tels quels. Ce qui est mis a
//! l'epreuve ici n'est pas ce qui est PERMIS -- cela, un systeme sans aucune
//! verification le fait aussi -- mais ce qui doit etre REFUSE.
//!
//! # Les deux defauts que ces tests ont attrapes
//!
//! 1. RequestServer etait classe `BrowserContent`, exactement comme
//!    WebContent. Les deux avaient donc la meme autorite, et l'architecture ou
//!    « le reseau appartient a RequestServer » ne pouvait pas etre appliquee :
//!    retirer le reseau au profil aurait casse RequestServer, le lui laisser
//!    l'aurait donne au moteur de rendu.
//!
//! 2. Seules les sockets BRUTES demandaient un droit. Toute autre socket etait
//!    ouverte a tout le monde -- un moteur de rendu compromis pouvait donc
//!    ouvrir une connexion TCP vers n'importe quel hote, ce qui transforme une
//!    faille d'analyse HTML en canal de sortie.

#![allow(dead_code)]

extern crate alloc;

mod capability {
    include!("../../src/kernel/security/capability.rs");
}
mod credentials {
    include!("../../src/kernel/security/credentials.rs");
}
mod profile {
    include!("../../src/kernel/security/profile.rs");
}

use capability::Capabilities;
use profile::{
    capabilities, classify, initial_capabilities, sandboxe, transition_capabilities,
    SecurityProfile,
};

const UTILISATEUR: u32 = 1000;
const ROOT: u32 = 0;

// --- Ce que le controle reseau fait, reproduit a l'identique ----------------

const AF_PACKET: u32 = 17;
const SOCK_TYPE_MASK: u32 = 0xf;
const SOCK_RAW: u32 = 3;
const SOCK_STREAM: u32 = 1;
const AF_INET: u32 = 2;

/// La regle de `security::network::socket_allowed`, sans le `Snapshot` que
/// seul le noyau sait construire.
fn socket_permise(caps: Capabilities, domaine: u32, genre: u32) -> bool {
    if !caps.contains(Capabilities::NET_CONNECT) {
        return false;
    }
    let brute = genre & SOCK_TYPE_MASK == SOCK_RAW || domaine == AF_PACKET;
    !brute || caps.contains(Capabilities::NETWORK_ADMIN)
}

/// La regle de `security::execution::appelant_peut_executer`.
fn appelant_peut_executer(profil: SecurityProfile) -> bool {
    matches!(
        profil,
        SecurityProfile::System | SecurityProfile::User | SecurityProfile::BrowserBroker
    )
}

fn caps_de(image: &str) -> Capabilities {
    initial_capabilities(image, UTILISATEUR)
}

// ---------------------------------------------------------------------------
// La classification : chaque role a le SIEN.
// ---------------------------------------------------------------------------

#[test]
fn chaque_role_du_navigateur_a_son_profil() {
    for image in [
        "/usr/libexec/ladybird/WebContent",
        "/usr/libexec/ladybird/WebWorker",
        "/usr/libexec/ladybird/ImageDecoder",
    ] {
        assert_eq!(
            classify(image, UTILISATEUR), SecurityProfile::BrowserContent,
            "{image} est un role de RENDU"
        );
    }
    assert_eq!(
        classify("/usr/libexec/ladybird/RequestServer", UTILISATEUR),
        SecurityProfile::BrowserNetwork,
        "RequestServer POSSEDE le reseau : le classer comme un rendu rendait la \
         regle inexprimable"
    );
    assert_eq!(
        classify("/usr/libexec/ladybird/BrowserHost", UTILISATEUR),
        SecurityProfile::BrowserBroker
    );
}

/// L'emplacement l'emporte sur le nom. Un binaire copie dans /tmp ne devient
/// pas RequestServer en s'appelant ainsi.
#[test]
fn l_emplacement_l_emporte_sur_le_nom() {
    assert_eq!(
        classify("/tmp/RequestServer", UTILISATEUR), SecurityProfile::Untrusted
    );
    assert_eq!(
        classify("/var/tmp/BrowserHost", ROOT), SecurityProfile::Untrusted
    );
}

// ---------------------------------------------------------------------------
// LE RESEAU : au seul proprietaire.
// ---------------------------------------------------------------------------

/// LE CAS QUI N'ETAIT PAS COUVERT : une socket TCP ordinaire depuis un moteur
/// de rendu.
#[test]
fn un_moteur_de_rendu_ne_peut_pas_ouvrir_de_socket() {
    let rendu = caps_de("/usr/libexec/ladybird/WebContent");
    assert!(
        !socket_permise(rendu, AF_INET, SOCK_STREAM),
        "un WebContent compromis pouvait parler a n'importe quel hote : c'est \
         une faille d'analyse HTML transformee en canal de sortie"
    );
    assert!(!socket_permise(rendu, AF_INET, SOCK_RAW));
    assert!(!socket_permise(rendu, AF_PACKET, SOCK_STREAM));
}

#[test]
fn le_serveur_de_requetes_garde_le_reseau() {
    let reseau = caps_de("/usr/libexec/ladybird/RequestServer");
    assert!(
        socket_permise(reseau, AF_INET, SOCK_STREAM),
        "RequestServer POSSEDE le reseau : le lui retirer casserait le navigateur"
    );
    assert!(
        !socket_permise(reseau, AF_INET, SOCK_RAW),
        "mais pas les sockets brutes : il n'a aucune raison de forger des paquets"
    );
    assert!(!socket_permise(reseau, AF_PACKET, SOCK_STREAM));
}

#[test]
fn les_roles_legitimes_gardent_leur_reseau() {
    for (image, uid) in [
        ("/usr/bin/curl", UTILISATEUR),
        ("/usr/libexec/ladybird/BrowserHost", UTILISATEUR),
        ("/usr/libexec/ladybird/BrowserHost", ROOT),
    ] {
        let caps = initial_capabilities(image, uid);
        assert!(
            socket_permise(caps, AF_INET, SOCK_STREAM),
            "{image} doit garder le reseau : une regle qui casse tout n'est pas \
             appliquee, elle est desactivee"
        );
    }
    // Root garde tout, sockets brutes comprises.
    let systeme = initial_capabilities("/usr/sbin/quelque-chose", ROOT);
    assert!(socket_permise(systeme, AF_INET, SOCK_RAW));
}

#[test]
fn un_binaire_non_fiable_n_a_pas_le_reseau() {
    let caps = caps_de("/tmp/charge-utile");
    assert!(!socket_permise(caps, AF_INET, SOCK_STREAM));
}

// ---------------------------------------------------------------------------
// L'EXEC : les deux moities de la question.
// ---------------------------------------------------------------------------

/// Le courtier doit pouvoir LANCER un WebContent. La capacite `EXEC` du profil
/// de la CIBLE est ce qui l'autorise, et elle doit rester.
#[test]
fn le_courtier_peut_lancer_un_moteur_de_rendu() {
    assert!(
        capabilities(SecurityProfile::BrowserContent).contains(Capabilities::EXEC),
        "sans EXEC sur l'image, le courtier ne pourrait plus lancer WebContent"
    );
    assert!(appelant_peut_executer(SecurityProfile::BrowserBroker));
}

/// Et le moteur de rendu, lui, ne doit rien pouvoir lancer. C'est l'autre
/// moitie de la question, et elle n'etait posee NULLE PART.
#[test]
fn un_moteur_de_rendu_ne_peut_rien_lancer() {
    for profil in [
        SecurityProfile::BrowserContent,
        SecurityProfile::BrowserNetwork,
        SecurityProfile::Untrusted,
    ] {
        assert!(
            !appelant_peut_executer(profil),
            "{profil:?} pouvait lancer n'importe quel binaire du systeme : la \
             verification ne regardait jamais QUI appelait"
        );
    }
}

#[test]
fn les_roles_legitimes_peuvent_encore_lancer() {
    assert!(appelant_peut_executer(SecurityProfile::System));
    assert!(appelant_peut_executer(SecurityProfile::User));
    assert!(appelant_peut_executer(SecurityProfile::BrowserBroker));
}

// ---------------------------------------------------------------------------
// LES PERIPHERIQUES.
// ---------------------------------------------------------------------------

#[test]
fn aucun_role_de_navigateur_hors_courtier_n_ouvre_de_peripherique() {
    for image in [
        "/usr/libexec/ladybird/WebContent",
        "/usr/libexec/ladybird/WebWorker",
        "/usr/libexec/ladybird/ImageDecoder",
        "/usr/libexec/ladybird/RequestServer",
    ] {
        assert!(
            !caps_de(image).contains(Capabilities::DEVICE_IO),
            "{image} ne doit pas pouvoir ouvrir un peripherique arbitraire"
        );
    }
    // Le courtier, lui, en a besoin : c'est lui qui tient l'affichage. Mais il
    // ne l'obtient que de l'autorite AMBIANTE de son lanceur : un courtier
    // lance par un utilisateur ordinaire ne gagne pas DEVICE_IO en s'appelant
    // BrowserHost.
    assert!(
        !caps_de("/usr/libexec/ladybird/BrowserHost").contains(Capabilities::DEVICE_IO),
        "un utilisateur ordinaire ne fabrique pas DEVICE_IO par le nom de son \
         binaire"
    );
    assert!(
        initial_capabilities("/usr/libexec/ladybird/BrowserHost", ROOT)
            .contains(Capabilities::DEVICE_IO),
        "lance par root, le courtier garde de quoi tenir l'affichage"
    );

    // Et meme lance par root, un role de RENDU ne l'obtient pas.
    assert!(
        !initial_capabilities("/usr/libexec/ladybird/WebContent", ROOT)
            .contains(Capabilities::DEVICE_IO)
    );
}

// ---------------------------------------------------------------------------
// L'ACQUISITION DE PRIVILEGES.
// ---------------------------------------------------------------------------

/// Aucun role sandboxe ne doit pouvoir changer d'identite ni administrer.
#[test]
fn aucun_role_sandboxe_n_acquiert_de_privilege() {
    for image in [
        "/usr/libexec/ladybird/WebContent",
        "/usr/libexec/ladybird/RequestServer",
        "/tmp/charge-utile",
    ] {
        let caps = caps_de(image);
        for interdit in [
            Capabilities::SET_IDENTITY,
            Capabilities::SYSTEM_ADMIN,
            Capabilities::FS_ADMIN,
            Capabilities::NETWORK_ADMIN,
            Capabilities::PROCESS_CONTROL,
            Capabilities::DEBUG,
            Capabilities::EXEC_UNTRUSTED,
        ] {
            assert!(
                !caps.contains(interdit),
                "{image} ne doit pas porter {interdit:?}"
            );
        }
    }
}

/// `no_new_privs` est pose PAR LE PROFIL, pas demande par le programme.
///
/// Le demander soi-meme suppose que le programme le fasse -- or c'est
/// precisement celui dont on suppose qu'il peut etre compromis.
#[test]
fn un_role_sandboxe_porte_no_new_privs_d_office() {
    assert!(sandboxe(SecurityProfile::BrowserContent));
    assert!(sandboxe(SecurityProfile::BrowserNetwork));
    assert!(sandboxe(SecurityProfile::Untrusted));
    assert!(!sandboxe(SecurityProfile::System));
    assert!(!sandboxe(SecurityProfile::User));
    assert!(
        !sandboxe(SecurityProfile::BrowserBroker),
        "le courtier lance des processus et tient l'affichage : le sandboxer \
         comme un rendu casserait le navigateur"
    );
}

/// Meme lance par root, un role de navigateur ne remonte pas.
#[test]
fn root_ne_releve_pas_un_role_sandboxe() {
    for image in [
        "/usr/libexec/ladybird/WebContent",
        "/usr/libexec/ladybird/RequestServer",
    ] {
        let caps = initial_capabilities(image, ROOT);
        assert!(!caps.contains(Capabilities::SYSTEM_ADMIN));
        assert!(!caps.contains(Capabilities::DEVICE_IO));
        assert!(
            !caps.contains(Capabilities::NETWORK_ADMIN),
            "{image} lance par root ne doit pas gagner l'administration reseau"
        );
    }
}

/// Une transition ne fabrique jamais d'autorite, quelle que soit sa direction.
#[test]
fn une_transition_ne_fabrique_jamais_de_reseau() {
    let rendu = caps_de("/usr/libexec/ladybird/WebContent");
    assert!(!rendu.contains(Capabilities::NET_CONNECT));

    // Le rendu tente de devenir RequestServer : il n'y gagne pas le reseau.
    let tentative = transition_capabilities(rendu, SecurityProfile::BrowserNetwork);
    assert!(
        !tentative.contains(Capabilities::NET_CONNECT),
        "un exec vers RequestServer ne doit pas donner le reseau a qui ne \
         l'avait pas"
    );

    // Et dans l'autre sens, RequestServer qui tombe vers le rendu le PERD.
    let reseau = caps_de("/usr/libexec/ladybird/RequestServer");
    let degrade = transition_capabilities(reseau, SecurityProfile::BrowserContent);
    assert!(!degrade.contains(Capabilities::NET_CONNECT));
}

/// `Capabilities::ALL` doit contenir le nouveau droit : un droit hors de ALL
/// serait invisible a root, et la regle « root peut tout » deviendrait fausse
/// sans que rien ne le dise.
#[test]
fn le_droit_reseau_fait_partie_de_toute_l_autorite() {
    assert!(Capabilities::ALL.contains(Capabilities::NET_CONNECT));
    assert!(capabilities(SecurityProfile::System).contains(Capabilities::NET_CONNECT));
}
