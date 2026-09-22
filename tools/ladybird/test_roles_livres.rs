//! LES DEUX TABLES QUI DECRIVENT LES PROCESSUS LADYBIRD DOIVENT S'ACCORDER.
//!
//! # L'invariant, tel que le code le documente deja
//!
//! `supervision_corps.rs` le dit de lui-meme, au-dessus de `depuis_image` :
//!
//! > La meme source que `security::profile::classify` : les deux doivent
//! > s'accorder, et un role connu ici mais pas la-bas serait un processus
//! > supervise sans etre sandboxe.
//!
//! Rien ne le verifiait. Les deux tables ont donc divergé, et de trois
//! facons -- mesurees le 22 septembre 2026 sur les chemins reellement livres
//! par `tools/ci/run_ladybird_browser_host.sh` :
//!
//! ```text
//!   image livree                                securite        supervision
//!   /bo-navigateur                              (defaut)        courtier
//!   /usr/libexec/ladybird/BouchaudBrowserHost   (defaut)        (AUCUN)
//!   /usr/libexec/ladybird/Compositor            BrowserBroker   (AUCUN)
//! ```
//!
//! Chacune a une consequence distincte :
//!
//!   * `/bo-navigateur` est le binaire que le bureau LANCE
//!     (`gui::client::CHEMIN_NAVIGATEUR`). La securite ne le reconnait pas,
//!     parce qu'elle exige le chemin exact `/usr/bin/bo-navigateur` -- et le
//!     binaire est deplie a la racine du RAMFS. La supervision avait ete
//!     corrigee pour cela, avec un commentaire qui l'explique ; la securite
//!     ne l'a pas ete. Le courtier tourne donc sans le profil qui lui donne
//!     le droit de lancer ses aides.
//!
//!   * `BouchaudBrowserHost` n'est reconnu par AUCUNE des deux. Le test
//!     `ends_with("/BrowserHost")` echoue sur `BouchaudBrowserHost` : le
//!     caractere qui precede n'est pas une barre oblique mais un `d`.
//!
//!   * `Compositor` recoit le profil PRIVILEGIE de courtier et n'a aucun role
//!     de supervision. C'est la combinaison la plus mauvaise des deux : il
//!     peut lancer des processus, et sa mort ne se voit nulle part -- ni dans
//!     Services, ni dans la politique de relance.
//!
//! # Ce que ce banc verifie
//!
//! Que chaque image reellement livree est connue des DEUX tables, et qu'aucune
//! ne recoit un profil privilegie sans etre supervisee.

#![allow(dead_code)]

extern crate alloc;

/// Les deux modules de production sont inclus TELS QUELS. Ce qui est remplace
/// ici ne touche pas la logique verifiee : un verrou d'hote a la place du
/// verrou du noyau, une trace serie qui ne va nulle part, et le jeu de
/// capacites -- que ce banc ne consulte pas, puisque sa question porte sur la
/// RECONNAISSANCE des images et non sur les droits qu'elle accorde.
mod kernel {
    pub mod sync {
        pub struct SpinLock<T> {
            inner: std::sync::Mutex<T>,
        }
        impl<T> SpinLock<T> {
            pub const fn new(valeur: T) -> Self {
                Self { inner: std::sync::Mutex::new(valeur) }
            }
            pub fn lock(&self) -> std::sync::MutexGuard<'_, T> {
                self.inner.lock().unwrap_or_else(|e| e.into_inner())
            }
        }
    }
}

use crate::kernel::sync::SpinLock;

#[macro_export]
macro_rules! serial_println {
    ($($arg:tt)*) => {{ let _ = format!($($arg)*); }};
}

mod securite {
    /// Le jeu de capacites REEL : `capability.rs` ne depend de rien, donc il
    /// s'inclut tel quel. Un jeu recopie ici divergerait, et ce banc verifie
    /// justement ce que coute la divergence de deux tables.
    pub mod capability {
        include!("../../src/kernel/security/capability.rs");
    }

    pub mod profile {
        include!("../../src/kernel/security/profile.rs");
    }
}

mod supervision {
    use super::SpinLock;
    use core::sync::atomic::{AtomicU64, Ordering};
    include!("../../src/kernel/navigateur/supervision_corps.rs");
}

use securite::profile::{classify, SecurityProfile};
use supervision::Role;

/// Les images que `tools/ci/run_ladybird_browser_host.sh` copie dans l'image,
/// aux chemins ou il les copie.
///
/// Cette liste est la SEULE chose que ce banc tient pour acquise, et elle est
/// verifiable : chaque entree se lit dans le script de scenario.
const LIVREES: &[&str] = &[
    "/bo-navigateur",
    "/usr/libexec/ladybird/BouchaudBrowserHost",
    "/usr/libexec/ladybird/WebContent",
    "/usr/libexec/ladybird/RequestServer",
    "/usr/libexec/ladybird/ImageDecoder",
    "/usr/libexec/ladybird/Compositor",
    "/usr/libexec/ladybird/WebWorker",
];

fn est_un_profil_de_navigateur(profil: SecurityProfile) -> bool {
    matches!(
        profil,
        SecurityProfile::BrowserBroker
            | SecurityProfile::BrowserContent
            | SecurityProfile::BrowserNetwork
    )
}

#[test]
fn chaque_image_livree_est_supervisee() {
    let mut inconnues = Vec::new();
    for image in LIVREES {
        if Role::depuis_image(image).is_none() {
            inconnues.push(*image);
        }
    }
    assert!(
        inconnues.is_empty(),
        "images livrees sans role de supervision : {:?}\n\
         Un processus sans role n'apparait pas dans Services, sa mort ne \
         declenche aucune politique, et personne ne sait s'il est pret.",
        inconnues
    );
}

#[test]
fn chaque_image_livree_a_un_profil_de_navigateur() {
    let mut sans_profil = Vec::new();
    for image in LIVREES {
        let profil = classify(image, 0);
        if !est_un_profil_de_navigateur(profil) {
            sans_profil.push((*image, profil));
        }
    }
    assert!(
        sans_profil.is_empty(),
        "images livrees sans profil de navigateur : {:?}\n\
         Le profil par defaut n'accorde pas les droits qu'un processus du \
         navigateur exige, et n'impose pas non plus le bac a sable qui lui \
         convient.",
        sans_profil
    );
}

#[test]
fn aucun_privilege_de_courtier_sans_supervision() {
    // LA REGLE LA PLUS IMPORTANTE DES TROIS.
    //
    // Le profil de courtier donne le droit de LANCER des processus. Un
    // processus qui l'obtient sans etre supervise peut en lancer d'autres
    // sans que rien ne compte ses relances, ni ne remarque sa mort.
    for image in LIVREES {
        if classify(image, 0) != SecurityProfile::BrowserBroker {
            continue;
        }
        assert!(
            Role::depuis_image(image).is_some(),
            "{} recoit le profil privilegie de courtier sans avoir de role \
             de supervision",
            image
        );
    }
}

#[test]
fn le_binaire_que_le_bureau_lance_est_un_courtier() {
    // `gui::client::CHEMIN_NAVIGATEUR`. Si cette constante change, ce test
    // doit changer avec elle -- et c'est precisement ce qui n'a pas eu lieu
    // la derniere fois qu'elle a change.
    const CHEMIN_NAVIGATEUR: &str = "/bo-navigateur";
    assert_eq!(Role::depuis_image(CHEMIN_NAVIGATEUR), Some(Role::Courtier));
    assert_eq!(classify(CHEMIN_NAVIGATEUR, 0), SecurityProfile::BrowserBroker);
}

#[test]
fn un_binaire_copie_dans_tmp_ne_devient_pas_courtier_par_son_nom() {
    // La regle qui existait deja et qu'il ne faut pas perdre en elargissant
    // la reconnaissance : l'emplacement l'emporte sur le nom.
    // Les trois prefixes que `untrusted_path` refuse. `/home/` n'en est pas
    // un et ne doit pas etre ajoute ici par analogie : Bouchaud OS n'a pas de
    // `/home`, et inventer une regle dans un test la rendrait vraie nulle part.
    for image in [
        "/tmp/BouchaudBrowserHost",
        "/tmp/bo-navigateur",
        "/var/tmp/Compositor",
        "/usr/share/Downloads/WebContent",
    ] {
        assert_eq!(
            classify(image, 0),
            SecurityProfile::Untrusted,
            "{} ne doit pas gagner de privilege par son nom",
            image
        );
    }
}
