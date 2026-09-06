//! Harnais de test hote du presse-papiers du bureau.
//!
//! `src/gui/presse_papiers.rs` est inclus tel quel : le code exerce ici est
//! exactement celui qui tourne sur la machine. Ses deux dependances -- le
//! plafond de charge du protocole et un verrou -- sont fournies par ce
//! fichier : le protocole est le VRAI module, inclus lui aussi, et seul le
//! verrou est remplace, par un `Mutex` de la bibliotheque standard.
//!
//! Ce choix delimite ce que ce harnais prouve. Il ne prouve rien sur
//! l'exclusion mutuelle -- le verrou n'est pas celui du noyau -- et tout sur
//! ce qui casse en silence :
//!
//!   * la BORNE. Un contenu qui depasse la capacite doit etre tronque et non
//!     pris tel quel : c'est ce qui empeche un client d'obliger le noyau a
//!     retenir ce qu'il veut. Le decodeur du protocole borne deja la charge
//!     utile, et cette seconde borne est la defense en profondeur -- celle qui
//!     tient encore le jour ou les deux valeurs divergent.
//!   * la GENERATION. C'est elle, et rien d'autre, qui decide si le
//!     gestionnaire de fenetres pousse le contenu a un client. Une generation
//!     qui n'avance pas laisse un client coller un texte perime ; une
//!     generation qui avance sans raison recopie quatre kibioctets par tour de
//!     composition.

#![allow(dead_code)]

extern crate alloc;

#[path = "../../src/gui/protocole.rs"]
pub mod protocole_reel;

/// Les chemins que `presse_papiers.rs` emprunte dans le noyau.
///
/// Ils sont reproduits ici plutot que modifies la-bas : le module teste doit
/// rester lisible comme du code de noyau, pas comme du code adapte a son
/// harnais.
mod gui {
    pub use crate::protocole_reel as protocole;
}

mod kernel {
    pub mod sync {
        /// Le verrou du noyau, remplace par celui de l'hote.
        ///
        /// Seule l'INTERFACE compte ici -- `const fn new` pour un `static`, et
        /// un `lock()` qui rend un garde deref-able. Ce que le harnais ne peut
        /// pas prouver est ecrit en tete de fichier.
        pub struct SpinLock<T>(std::sync::Mutex<T>);

        impl<T> SpinLock<T> {
            pub const fn new(valeur: T) -> Self {
                SpinLock(std::sync::Mutex::new(valeur))
            }

            pub fn lock(&self) -> std::sync::MutexGuard<'_, T> {
                // Un verrou empoisonne signifie qu'un test a panique en le
                // tenant : le masquer donnerait un second echec sans rapport.
                self.0.lock().expect("verrou du presse-papiers empoisonne")
            }
        }
    }
}

#[path = "../../src/gui/presse_papiers.rs"]
mod presse_papiers;

#[cfg(test)]
mod essais {
    use super::presse_papiers::{self, CAPACITE};

    /// Les cas partagent un `static` : ils s'executent donc en serie, sous ce
    /// verrou-ci. Le laisser au hasard de l'ordonnancement de `libtest`
    /// donnerait un harnais qui echoue une fois sur dix.
    static SERIE: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn serialise() -> std::sync::MutexGuard<'static, ()> {
        SERIE.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn ecrire_puis_lire_rend_le_meme_contenu() {
        let _serie = serialise();
        presse_papiers::ecrit(b"https://example.com/");
        let (octets, _) = presse_papiers::lit();
        assert_eq!(octets, b"https://example.com/");
    }

    #[test]
    fn la_generation_avance_a_chaque_ecriture() {
        let _serie = serialise();
        let avant = presse_papiers::ecrit(b"un");
        let apres = presse_papiers::ecrit(b"deux");
        assert!(
            apres > avant,
            "la generation n'a pas avance : un client qui a deja recu {avant} \
             ne se verrait jamais pousser « deux »"
        );
    }

    #[test]
    fn reecrire_le_meme_contenu_avance_quand_meme() {
        // Deliberement : le contenu n'est pas compare. Un client qui vient de
        // perdre puis de reprendre le foyer doit pouvoir etre resynchronise
        // sans que le bureau ait a se souvenir de ce qu'il possede.
        let _serie = serialise();
        let avant = presse_papiers::ecrit(b"identique");
        let apres = presse_papiers::ecrit(b"identique");
        assert!(apres > avant);
    }

    #[test]
    fn generation_ne_lit_pas_le_contenu() {
        let _serie = serialise();
        let attendue = presse_papiers::ecrit(b"contenu");
        assert_eq!(presse_papiers::generation(), attendue);
    }

    #[test]
    fn un_contenu_trop_grand_est_tronque() {
        let _serie = serialise();
        let enorme = vec![b'A'; CAPACITE * 3 + 17];
        presse_papiers::ecrit(&enorme);
        let (octets, _) = presse_papiers::lit();
        assert_eq!(
            octets.len(),
            CAPACITE,
            "le presse-papiers a retenu plus que sa capacite : un client \
             pourrait faire grossir la memoire du noyau a volonte"
        );
        assert!(octets.iter().all(|&o| o == b'A'));
    }

    #[test]
    fn un_contenu_exactement_a_la_capacite_passe_entier() {
        let _serie = serialise();
        let plein = vec![b'B'; CAPACITE];
        presse_papiers::ecrit(&plein);
        let (octets, _) = presse_papiers::lit();
        assert_eq!(octets.len(), CAPACITE, "la borne a rogne un octet de trop");
    }

    #[test]
    fn un_contenu_vide_est_un_contenu() {
        // Effacer le presse-papiers est une operation legitime, et elle doit
        // se propager comme une autre : sans avancee de generation, le client
        // garderait ce qu'il avait.
        let _serie = serialise();
        presse_papiers::ecrit(b"quelque chose");
        let avant = presse_papiers::generation();
        presse_papiers::ecrit(b"");
        let (octets, apres) = presse_papiers::lit();
        assert!(octets.is_empty());
        assert!(apres > avant);
    }

    #[test]
    fn la_capacite_est_celle_du_protocole() {
        // Deux bornes independantes pour une seule contrainte finissent par
        // diverger, et c'est alors le message qui est refuse -- ou pire,
        // accepte et tronque des deux cotes differemment.
        assert_eq!(CAPACITE, crate::protocole_reel::CHARGE_MAX as usize);
    }
}
