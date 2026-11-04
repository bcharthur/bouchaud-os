//! Politique de l'ecriture differee, PURE (BOUCHAUD_ECRITURE_DIFFEREE_V1).
//!
//! # Le manque
//!
//! Rien de ce qui etait ecrit sous `/persist` n'atteignait le disque sans un
//! `fsync` ou un `sync` explicite. Les programmes POSIX soigneux le font ; un
//! utilisateur qui cree un dossier dans Fichiers, un script du shell, une
//! application qui oublie `fsync`, perdaient tout a la premiere coupure.
//!
//! # La regle
//!
//! Une modification sous `/persist` fait monter une GENERATION. Le fil
//! d'ecriture differee ecrit quand :
//!
//!   * la generation n'a plus bouge depuis `CALME_MS` (une rafale d'ecritures
//!     -- un navigateur qui pose son cache -- donne UNE transaction, pas cent) ;
//!   * ou la premiere modification non ecrite date de `PLAFOND_MS` (une
//!     ecriture continue ne repousse pas indefiniment la durabilite).
//!
//! Une transaction qui echoue est retentee avec un delai qui double, borne :
//! un disque absent ou plein ne fait pas tourner le fil en boucle.
//!
//! La borne de perte apres une coupure est donc `PLAFOND_MS` (+ la duree d'une
//! transaction) pour tout ce qui n'a pas ete `fsync`e -- et zero pour ce qui
//! l'a ete, comme avant.

/// Calme demande avant d'ecrire, en millisecondes.
pub const CALME_MS: u64 = 1_500;
/// Retard maximal d'une modification non ecrite.
pub const PLAFOND_MS: u64 = 8_000;
/// Delai apres un echec, puis doublement jusqu'a `ECHEC_MAX_MS`.
pub const ECHEC_MS: u64 = 2_000;
pub const ECHEC_MAX_MS: u64 = 60_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Etat {
    /// Generation que le disque porte (derniere transaction reussie).
    pub ecrite: u64,
    /// Derniere generation observee, et quand elle l'a ete.
    pub vue: u64,
    pub vue_ms: u64,
    /// Premiere observation d'une generation non ecrite.
    pub retard_depuis_ms: Option<u64>,
    /// Pas de tentative avant cet instant (apres un echec).
    pub reprise_ms: u64,
    pub echec_ms: u64,
}

impl Etat {
    pub const fn neuf(generation: u64) -> Self {
        Self { ecrite: generation, vue: generation, vue_ms: 0, retard_depuis_ms: None,
            reprise_ms: 0, echec_ms: ECHEC_MS }
    }

    /// Observe `generation` a `maintenant` ; rend `true` s'il faut ecrire.
    pub fn decide(&mut self, generation: u64, maintenant: u64) -> bool {
        if generation == self.ecrite {
            self.retard_depuis_ms = None;
            self.vue = generation;
            return false;
        }
        if generation != self.vue {
            self.vue = generation;
            self.vue_ms = maintenant;
        }
        let depuis = *self.retard_depuis_ms.get_or_insert(maintenant);
        if maintenant < self.reprise_ms {
            return false;
        }
        maintenant.saturating_sub(self.vue_ms) >= CALME_MS
            || maintenant.saturating_sub(depuis) >= PLAFOND_MS
    }

    /// Une transaction qui avait capture `generation` a reussi.
    pub fn reussie(&mut self, generation: u64) {
        self.ecrite = generation;
        self.echec_ms = ECHEC_MS;
        self.reprise_ms = 0;
        if self.vue == generation {
            self.retard_depuis_ms = None;
        }
    }

    /// Une transaction a echoue a `maintenant`.
    pub fn echouee(&mut self, maintenant: u64) {
        self.reprise_ms = maintenant + self.echec_ms;
        self.echec_ms = (self.echec_ms * 2).min(ECHEC_MAX_MS);
    }
}
