//! La plus proche echeance de reveil, en une seule valeur atomique.
//!
//! # Ce que cela remplace
//!
//! `wake_sleepers()` balayait toute la table des taches, et `schedule()`
//! l'appelle a chaque tour.
//!
//! Une tache bloquee -- un fil arrete sur un futex, un `poll` en attente --
//! reste dans sa boucle `while Blocked { schedule() }` : elle s'endort au
//! `hlt`, le tick la reveille une milliseconde plus tard, elle reprend le gros
//! verrou, appelle `schedule()`, qui balaie toute la table, puis se rendort.
//!
//! Avec quatre fils de Ladybird bloques -- ce que `[SMP-STALL]` montrait, deux
//! CPU dans `futex` pendant plus de cent secondes d'affilee --, cela fait
//! quatre mille balayages complets par seconde, chacun sous le gros verrou,
//! pour ne rien trouver.
//!
//! # Le raccourci, et pourquoi il est sur
//!
//! Une borne INFERIEURE de la plus proche echeance. Tant que l'heure ne l'a pas
//! atteinte, aucune tache ne peut etre due : le balayage est inutile.
//!
//! Le sens de l'inegalite est tout. La borne peut etre TROP TOT -- on balaie
//! pour rien, ce qui coute mais ne perd rien --, jamais trop tard. Deux regles
//! suffisent a le garantir :
//!
//!   * toute pose d'echeance la ramene vers le passe (`arme`, un `fetch_min`) ;
//!   * tout balayage reel la recale sur le minimum exact (`recale`).
//!
//! Un retrait d'echeance ne la touche pas : il ne peut que rendre le vrai
//! minimum PLUS TARD, donc la borne reste inferieure. C'est ce qui permet de ne
//! rien avoir a faire sur les nombreux chemins qui remettent
//! `wake_deadline_ns` a zero.
//!
//! # Pourquoi ce module est separe
//!
//! Parce que la propriete se verifie sans ordonnanceur : `tools/smp/test_echeances.rs`
//! rejoue des sequences d'armements, de retraits et de balayages, et verifie
//! qu'aucune echeance due n'est jamais sautee.

use core::sync::atomic::{AtomicU64, Ordering};

/// « Aucune echeance en vue. »
pub const JAMAIS: u64 = u64::MAX;

pub struct Echeances {
    prochaine: AtomicU64,
}

impl Echeances {
    pub const fn neuve() -> Self {
        Self { prochaine: AtomicU64::new(JAMAIS) }
    }

    /// Declare une echeance. Zero veut dire « pas d'echeance » et n'arme rien.
    ///
    /// A appeler POUR CHAQUE `wake_deadline_ns` non nul : une echeance inconnue
    /// de la borne ne serait jamais servie par le raccourci.
    pub fn arme(&self, deadline_ns: u64) {
        if deadline_ns == 0 {
            return;
        }
        self.prochaine.fetch_min(deadline_ns, Ordering::Relaxed);
    }

    /// Faut-il balayer la table ?
    ///
    /// Rend `false` uniquement quand la borne prouve qu'aucune echeance n'est
    /// due. Un `true` de trop ne coute qu'un balayage.
    pub fn doit_balayer(&self, maintenant_ns: u64) -> bool {
        maintenant_ns >= self.prochaine.load(Ordering::Relaxed)
    }

    /// Recale la borne sur le minimum EXACT, apres un balayage complet.
    pub fn recale(&self, minimum_ns: u64) {
        self.prochaine.store(minimum_ns, Ordering::Relaxed);
    }

    /// La borne courante. Pour les diagnostics et les tests.
    pub fn borne(&self) -> u64 {
        self.prochaine.load(Ordering::Relaxed)
    }
}

impl Default for Echeances {
    fn default() -> Self {
        Self::neuve()
    }
}
