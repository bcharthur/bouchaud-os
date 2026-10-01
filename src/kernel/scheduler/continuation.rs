//! Qui peut reprendre la continuation d'un lancement synchrone.
//!
//! BOUCHAUD_CONTINUATION_SYNCHRONE_V1
//!
//! `run` / `run_noyau` garent la pile d'amorcage qui les a appeles, puis
//! commutent vers une racine. Cette pile ne doit etre reprise QUE lorsque la
//! racine est terminee, et sur le coeur qui l'a garee. Une tache quelconque
//! qui meurt sans successeur pret va a la boucle idle de son coeur.
//!
//! Regle pure, sans materiel ni etat global : le noyau l'applique dans
//! `exit_current` et dans la boucle idle, et `tools/smp/test_continuation.rs`
//! la rejoue contre l'ancien protocole.

/// Ou va la pile d'une tache qui vient de mourir.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Destination {
    /// La continuation du lancement synchrone : cette mort a termine sa racine.
    Continuation,
    /// Une autre tache prete de ce coeur.
    AutreTache,
    /// La boucle idle de ce coeur.
    Idle,
}

/// Ce que la sortie sait au moment de choisir.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sortie {
    /// Coeur ou la tache meurt.
    pub cpu: usize,
    /// Une continuation est-elle garee ?
    pub garee: bool,
    /// Coeur qui l'a garee.
    pub cpu_continuation: usize,
    /// La racine (et, pour `run`, sa descendance) est-elle terminee, cette
    /// mort comprise ?
    pub racine_terminee: bool,
    /// Une autre tache est-elle prete sur ce coeur ?
    pub autre_prete: bool,
}

/// La continuation peut-elle etre reprise par ce coeur, maintenant ?
///
/// La SEULE porte vers la continuation, pour la mort comme pour l'idle.
pub fn reprenable(garee: bool, racine_terminee: bool, cpu: usize, cpu_continuation: usize) -> bool {
    garee && racine_terminee && cpu == cpu_continuation
}

/// La destination d'une pile condamnee.
pub fn destination(s: Sortie) -> Destination {
    if reprenable(s.garee, s.racine_terminee, s.cpu, s.cpu_continuation) {
        Destination::Continuation
    } else if s.autre_prete {
        Destination::AutreTache
    } else {
        Destination::Idle
    }
}

/// Faut-il reveiller le coeur proprietaire ? Oui quand la continuation est
/// due mais que la mort a lieu ailleurs : seule sa boucle idle peut la
/// reprendre.
pub fn reveiller_proprietaire(s: Sortie) -> bool {
    s.garee && s.racine_terminee && s.cpu != s.cpu_continuation
}
