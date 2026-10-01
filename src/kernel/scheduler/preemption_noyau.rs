//! La DECISION de preempter un fil noyau depuis une interruption, pure.
//!
//! BOUCHAUD_PREEMPTION_NOYAU_SURE_V1
//!
//! Le noyau n'est pas preemptible en son sein : plusieurs sous-systemes en
//! font leur contrat (`thread.rs`, `registre.rs`, `smp.rs`,
//! `window_manager.rs`). Un fil noyau ne quitte son coeur qu'a ses points
//! volontaires (`schedule`, sommeil) -- et, depuis
//! BOUCHAUD_P0_REVEIL_CIBLE_V1, sur DEMANDE CIBLEE : le reveil d'une tache
//! sensible a la latence peut faire commuter, depuis le tic, le fil noyau qui
//! occupe son coeur.
//!
//! Cette commutation n'est accordee que si le contexte interrompu est SUR :
//!
//!   * aucune section non preemptable (`preempt_count`) ;
//!   * aucun verrou simple, aucun verrou suivi par lockdep ;
//!   * AUCUNE LECTURE DU REGISTRE DES TACHES : son garde (`tasks()`) n'est
//!     ni un verrou simple ni un verrou suivi ; son contrat est qu'il ne
//!     traverse pas une commutation. Commute, il laisse le compte de son
//!     coeur a un pour la tache suivante, et l'ecrivain peut recycler un
//!     emplacement sous les yeux du lecteur suspendu ;
//!   * AUCUN SHOOTDOWN TLB EN VOL depuis ce coeur : un seul par coeur (son
//!     emplacement est indexe par le coeur), attendu avec l'IF de l'appelant ;
//!   * pas de sortie de tache en cours (BOUCHAUD_SORTIE_NON_PREEMPTEE_V1) ;
//!   * interruptions MASQUEES au moment de la decision (l'IRQ imbriquee, plus
//!     bas).
//!
//! # Le quantum noyau, retire
//!
//! BOUCHAUD_QUANTUM_NOYAU_V1 (`1df8a88b`) accordait aussi la commutation a
//! tout fil noyau ayant tourne deux quanta devant des taches pretes. Ce
//! n'etait pas un nouveau motif, c'etait un nouveau DOMAINE : n'importe quel
//! point IF=1 sans verrou compte de n'importe quel fil noyau -- precisement
//! les points que les contrats ci-dessus supposent ininterrompus. Mesure :
//! `task: runtime > fenetre tid=104 delta=33563262577` (panic,
//! scheduler-ng-banc SMP8, QEMU), dans `services-metrics` parcourant la
//! table sous garde de lecture. Un fil noyau qui garde son coeur trop
//! longtemps se traite par des points cooperatifs dans sa boucle, pas en le
//! coupant n'importe ou.
//!
//! # L'IRQ imbriquee
//!
//! Commuter depuis une interruption qui en interrompt une autre suspendrait
//! le gestionnaire exterieur au milieu de son travail -- avant son EOI peut-
//! etre, ce qui bloquerait sur ce coeur toute interruption de priorite
//! inferieure ou egale. Ce cas est exclu PAR CONSTRUCTION :
//!
//!   1. toutes les entrees de l'IDT sont des portes d'interruption (IF remis
//!      a zero par le processeur a l'entree) -- `set_handler_fn`, jamais de
//!      porte de trappe ;
//!   2. aucun chemin execute depuis un gestionnaire ne remet IF a un : les
//!      `sti` / `interrupts::enable()` du noyau sont tous en contexte de
//!      tache, d'idle ou d'amorcage, et les verrous `SpinLockIrq` ne rendent
//!      IF que s'il etait leve a la prise.
//!
//! `tools/verifie-preemption-noyau.py` verifie les deux. La condition
//! `interruptions_ouvertes == false` est leur controle a l'execution.
//!
//! Module sans dependance : compile tel quel dans le noyau et dans le test
//! hote `tools/smp/test_preemption_noyau.rs`.

/// Ce que la decision lit du coeur, au tic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Contexte {
    pub demande_ciblee: bool,
    pub preempt_count: u32,
    pub verrous_simples: u32,
    pub profondeur_lockdep: u32,
    pub lectures_registre: u32,
    pub shootdown_en_vol: bool,
    pub sortie_en_cours: bool,
    pub interruptions_ouvertes: bool,
}

/// Le contexte interrompu permet-il de commuter ?
pub fn contexte_sur(c: &Contexte) -> bool {
    c.preempt_count == 0
        && c.verrous_simples == 0
        && c.profondeur_lockdep == 0
        && c.lectures_registre == 0
        && !c.shootdown_en_vol
        && !c.sortie_en_cours
        && !c.interruptions_ouvertes
}

/// La decision : une demande ciblee, dans un contexte sur. Rien d'autre.
pub fn decide(c: &Contexte) -> bool {
    c.demande_ciblee && contexte_sur(c)
}
