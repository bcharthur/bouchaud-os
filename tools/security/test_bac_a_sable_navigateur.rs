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
mod chemins {
    include!("../../src/kernel/security/chemins.rs");
}

use capability::Capabilities;
use chemins::{
    ecriture_permise, lecture_permise, DOSSIER_TELECHARGEMENTS, MAGASIN_DU_CHROME,
    PROFIL_NAVIGATEUR,
};
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

// ---------------------------------------------------------------------------
// Le profil persistant : qui y ecrit, et surtout qui n'y ecrit pas
//
// BOUCHAUD_C19_PROFIL_PERSISTANT_DU_NAVIGATEUR
//
// La couche plateforme du portage demandait un profil sur `/persist`, et le bac
// a sable ne connaissait que `/tmp`. RequestServer se voyait donc refuser son
// propre magasin -- soixante-cinq refus par session, aucun cache HTTP, aucun
// HSTS conserve d'un demarrage a l'autre.
//
// Ce que ces tests gardent n'est pas l'autorisation. C'est le REFUS : que
// l'ouverture faite pour RequestServer ne s'etende pas au moteur de rendu, qui
// est le processus qu'un site hostile atteint en premier. Un rendu compromis
// qui pourrait ecrire sur `/persist` survivrait a un redemarrage.
// ---------------------------------------------------------------------------

const CACHE_ALT_SVC: &str = "/persist/ladybird/profile/cache/alt-svc-cache.txt";

#[test]
fn le_serveur_de_requetes_possede_son_profil_persistant() {
    // Le chemin exact que le journal montrait refuse, soixante-cinq fois.
    assert!(lecture_permise(SecurityProfile::BrowserNetwork, CACHE_ALT_SVC));
    assert!(ecriture_permise(SecurityProfile::BrowserNetwork, CACHE_ALT_SVC));
    assert!(ecriture_permise(
        SecurityProfile::BrowserNetwork,
        "/persist/ladybird/data/cookies.sqlite"
    ));
    assert!(ecriture_permise(SecurityProfile::BrowserNetwork, PROFIL_NAVIGATEUR));
}

#[test]
fn un_moteur_de_rendu_n_ecrit_rien_de_persistant() {
    // LE test de ce chantier. WebContent, WebWorker et ImageDecoder analysent
    // ce qui vient du reseau : leur donner un octet d'ecriture persistante
    // transformerait une faille d'analyse en implantation durable.
    for chemin in [
        CACHE_ALT_SVC,
        "/persist/ladybird/data/cookies.sqlite",
        PROFIL_NAVIGATEUR,
        "/persist",
        // Voisin de nom du dossier de telechargement, qui lui EST ouvert :
        // c'est exactement la ou un prefixe mal compare ferait un trou.
        "/persist/Downloads-vole/charge.exe",
    ] {
        assert!(
            !ecriture_permise(SecurityProfile::BrowserContent, chemin),
            "un role de rendu ne doit pas pouvoir ecrire {}",
            chemin
        );
        assert!(
            !lecture_permise(SecurityProfile::BrowserContent, chemin),
            "un role de rendu ne doit meme pas pouvoir lire {}",
            chemin
        );
    }
}

#[test]
fn un_moteur_de_rendu_depose_ses_telechargements_et_rien_d_autre() {
    // BOUCHAUD_C20_TELECHARGEMENTS
    //
    // Ce droit est un ELARGISSEMENT, et le test le dit dans les deux sens : ce
    // qu'il ouvre, et ce qu'il n'ouvre pas. Le second compte davantage --
    // c'est lui qui echouera le jour ou quelqu'un elargira le predicat en
    // croyant simplifier.
    assert!(ecriture_permise(
        SecurityProfile::BrowserContent,
        "/persist/Downloads/rapport.pdf"
    ));
    assert!(lecture_permise(
        SecurityProfile::BrowserContent,
        "/persist/Downloads/rapport.pdf"
    ));
    // Le dossier lui-meme : le portage y fait un `mkdir` au demarrage, et un
    // sous-arbre qui exclurait sa propre racine echouerait a la creer.
    assert!(ecriture_permise(
        SecurityProfile::BrowserContent,
        DOSSIER_TELECHARGEMENTS
    ));

    // La frontiere qui compte : le PROFIL du navigateur -- cookies, HSTS,
    // cache -- reste ferme au rendu. Ce qu'il gagne est un depot, pas une
    // memoire.
    for chemin in [
        CACHE_ALT_SVC,
        "/persist/ladybird/data/cookies.sqlite",
        PROFIL_NAVIGATEUR,
        "/persist",
        "/persist/autre/charge",
    ] {
        assert!(
            !ecriture_permise(SecurityProfile::BrowserContent, chemin),
            "le depot de telechargement a elargi {} au passage",
            chemin
        );
    }

    // Le droit est attache au ROLE. RequestServer lit et ecrit le profil, pas
    // le depot : c'est WebContent qui tient les octets du corps de reponse.
    assert!(!ecriture_permise(
        SecurityProfile::BrowserNetwork,
        "/persist/Downloads/rapport.pdf"
    ));
    assert!(!ecriture_permise(
        SecurityProfile::Untrusted,
        "/persist/Downloads/rapport.pdf"
    ));
    assert!(!lecture_permise(
        SecurityProfile::Untrusted,
        "/persist/Downloads/rapport.pdf"
    ));

    // Et le reste du bac a sable n'a pas bouge.
    for chemin in ["/usr/bin/sh", "/etc/passwd", "/root/.ssh/id_rsa", "/dev/fb0"] {
        assert!(
            !ecriture_permise(SecurityProfile::BrowserContent, chemin),
            "{} ne doit pas devenir inscriptible",
            chemin
        );
    }
}

#[test]
fn le_magasin_du_chrome_est_un_voisin_de_nom_et_pas_un_descendant() {
    // BOUCHAUD_C21_HISTORIQUE_ET_FAVORIS
    //
    // `/persist/ladybird-chrome` est volontairement voisin de
    // `/persist/ladybird`. C'est le cas exact ou une comparaison de prefixe
    // sans separateur transforme un droit en trou -- dans les DEUX sens.
    assert!(ecriture_permise(
        SecurityProfile::BrowserContent,
        "/persist/ladybird-chrome/favoris"
    ));
    assert!(lecture_permise(
        SecurityProfile::BrowserContent,
        "/persist/ladybird-chrome/historique"
    ));

    // Le rendu ne gagne pas le profil au passage.
    assert!(!ecriture_permise(
        SecurityProfile::BrowserContent,
        "/persist/ladybird/data/cookies.sqlite"
    ));
    assert!(!lecture_permise(
        SecurityProfile::BrowserContent,
        "/persist/ladybird/data/cookies.sqlite"
    ));

    // Et RequestServer ne gagne pas le magasin du chrome : le profil et le
    // magasin sont deux sous-arbres, deux roles, deux droits.
    assert!(!ecriture_permise(
        SecurityProfile::BrowserNetwork,
        "/persist/ladybird-chrome/favoris"
    ));
    assert!(!lecture_permise(
        SecurityProfile::Untrusted,
        "/persist/ladybird-chrome/favoris"
    ));

    // Un troisieme voisin n'existe pas.
    assert!(!ecriture_permise(
        SecurityProfile::BrowserContent,
        "/persist/ladybird-chrome-vole/favoris"
    ));
    assert!(!ecriture_permise(SecurityProfile::BrowserContent, MAGASIN_DU_CHROME.trim_end_matches("-chrome")));
}

#[test]
fn un_binaire_non_fiable_reste_dehors() {
    for chemin in [CACHE_ALT_SVC, PROFIL_NAVIGATEUR, "/persist"] {
        assert!(!lecture_permise(SecurityProfile::Untrusted, chemin));
        assert!(!ecriture_permise(SecurityProfile::Untrusted, chemin));
    }
}

#[test]
fn la_racine_du_volume_persistant_n_est_inscriptible_par_personne() {
    // `/persist` est LISIBLE pour RequestServer -- un magasin qui verifie
    // d'abord que son volume existe echouerait sinon avant d'atteindre son
    // propre sous-arbre -- mais sa racine appartient a la couche plateforme,
    // qui n'est pas sandboxee.
    assert!(lecture_permise(SecurityProfile::BrowserNetwork, "/persist"));
    assert!(!ecriture_permise(SecurityProfile::BrowserNetwork, "/persist"));
    assert!(!ecriture_permise(SecurityProfile::BrowserNetwork, "/persist/Downloads"));
    assert!(!ecriture_permise(SecurityProfile::BrowserNetwork, "/persist/autre"));
}

#[test]
fn un_voisin_de_nom_n_est_pas_un_descendant() {
    // La comparaison va jusqu'au separateur. Sans cela, `/persist/ladybird-vole`
    // passerait pour un descendant de `/persist/ladybird`, et le prefixe
    // accorde ouvrirait ses voisins de nom -- la facon classique de
    // transformer une autorisation en trou.
    assert!(!ecriture_permise(
        SecurityProfile::BrowserNetwork,
        "/persist/ladybird-vole/charge"
    ));
    assert!(!lecture_permise(
        SecurityProfile::BrowserNetwork,
        "/persist/ladybird-vole/charge"
    ));
    // Et la reciproque, pour que le test ne passe pas en refusant tout.
    assert!(ecriture_permise(
        SecurityProfile::BrowserNetwork,
        "/persist/ladybird/x"
    ));
}

#[test]
fn le_profil_persistant_n_ouvre_rien_d_autre() {
    // Le droit accorde a RequestServer porte sur UN sous-arbre. Il ne doit pas
    // avoir elargi le reste du bac a sable au passage.
    for chemin in ["/usr/bin/sh", "/etc/passwd", "/root/.ssh/id_rsa", "/dev/fb0"] {
        assert!(
            !ecriture_permise(SecurityProfile::BrowserNetwork, chemin),
            "{} ne doit pas devenir inscriptible",
            chemin
        );
    }
    // Ce qui etait deja lisible le reste, pour tous les roles sandboxes.
    for profil in [
        SecurityProfile::BrowserNetwork,
        SecurityProfile::BrowserContent,
        SecurityProfile::Untrusted,
    ] {
        assert!(lecture_permise(profil, "/usr/share/ladybird/fonts"));
        assert!(ecriture_permise(profil, "/tmp/ladybird-runtime/socket"));
    }
}
