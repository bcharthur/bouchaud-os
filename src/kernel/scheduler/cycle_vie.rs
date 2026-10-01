//! La machine d'etat du cycle de vie d'une tache.
//!
//! BOUCHAUD_CYCLE_DE_VIE_V1
//!
//! # Les trois etats, et qui a le droit d'ecrire quoi
//!
//! ```text
//!            endort (la tache elle-meme, CAS)
//!     Pret ─────────────────────────────────▶ Bloque
//!       ▲  ◀─────────────────────────────────   │
//!       │    reveille (un reveilleur, CAS)      │
//!       │    annule (la tache elle-meme, CAS)   │
//!       │                                       │ tue_parquee (un tueur, CAS,
//!       │ meurt (la tache elle-meme)            │  attente interruptible et
//!       ▼                                       ▼  tache hors de tout coeur)
//!     Zombie ◀──────────────────────────────────┘
//!       │
//!       └── absorbant : aucune transition n'en sort.
//! ```
//!
//! Zombie est ABSORBANT. Avant ce module, l'etat s'ecrivait par un simple
//! store (`range`) : une tache tuee par un frere (`exit_group`) pendant
//! qu'elle entrait dans `nanosleep` ecrasait `Zombie` par `Bloque`, l'echeance
//! la remettait `Pret`, et le fil d'un processus deja recolte retournait en
//! espace utilisateur. Mesure (scheduler-ng-banc, SMP2/4) : 210 a 266
//! compteurs de fils morts qui bougeaient encore apres la recolte.
//!
//! # Tuer une tache qui s'execute
//!
//! Une tache qui tourne -- en espace utilisateur ou dans le noyau -- ne peut
//! pas etre arretee de l'exterieur sans risque : elle peut tenir un verrou
//! dormant, une reference, une operation d'entree-sortie. Le tueur la
//! CONDAMNE (drapeau par tache) et elle meurt d'elle-meme a sa prochaine
//! frontiere : retour d'appel systeme, retour de faute, preemption depuis
//! l'espace utilisateur, premier passage en espace utilisateur, ou attente
//! INTERRUPTIBLE. Une attente non interruptible (verrou dormant, entree-sortie
//! du noyau) se termine normalement : sa fin ramene la tache a sa frontiere.
//!
//! Le tueur ne met lui-meme `Zombie` qu'a une tache PARQUEE dans une attente
//! interruptible : elle ne tient rien, et rien d'autre ne la reveillerait.
//!
//! # La fenetre du parking
//!
//! Le tueur ecrit `condamnee` puis lit l'etat ; la tache publie `Bloque` puis
//! relit `condamnee`. Ordre sequentiel des deux cotes (`SeqCst`) : au moins
//! l'un voit l'autre. Si la tache voit la condamnation, elle meurt sur place
//! (attente interruptible). Sinon le tueur voit `Bloque` : tache encore sur
//! son coeur -> il la REVEILLE (elle reboucle et voit la condamnation) ;
//! tache parquee -> il la tue sur place.
//!
//! `tools/smp/test_cycle_vie.rs` explore toutes les entrelacements du tueur,
//! de la tache et d'un reveilleur, contre l'ancien protocole et le nouveau.
//!
//! Regle pure, sans materiel ni etat global.

/// Les etats, avec les codes de `TaskState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Etat {
    Pret = 0,
    Bloque = 1,
    Zombie = 2,
}

/// Les transitions nommees. Ce sont les SEULES qui existent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition {
    /// La tache se declare bloquee : `Pret -> Bloque`.
    Endort,
    /// Un reveilleur, ou la tache qui annule son parking : `Bloque -> Pret`.
    Reveille,
    /// La tache meurt : `Pret | Bloque -> Zombie`.
    Meurt,
    /// Un tueur tue une tache parquee : `Bloque -> Zombie`.
    TueParquee,
}

/// L'etat d'arrivee, ou `None` si la transition est interdite depuis `depuis`.
pub const fn applique(depuis: Etat, t: Transition) -> Option<Etat> {
    match (depuis, t) {
        (Etat::Pret, Transition::Endort) => Some(Etat::Bloque),
        (Etat::Bloque, Transition::Reveille) => Some(Etat::Pret),
        (Etat::Pret, Transition::Meurt) | (Etat::Bloque, Transition::Meurt) => Some(Etat::Zombie),
        (Etat::Bloque, Transition::TueParquee) => Some(Etat::Zombie),
        _ => None,
    }
}

/// Ce que le tueur voit d'une tache, apres avoir publie sa condamnation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VueTueur {
    pub etat: Etat,
    /// La tache occupe un coeur (`on_cpu >= 0`) ou n'a pas fini de le
    /// quitter (`switching_out`).
    pub sur_coeur: bool,
    /// Son attente en cours, si elle est bloquee, est interruptible.
    pub interruptible: bool,
}

/// Ce que fait le tueur.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionTueur {
    /// Deja morte.
    Rien,
    /// Elle s'execute, ou elle est prete : elle mourra a sa frontiere. Si
    /// elle tourne sur un autre coeur, l'appelant le force a entrer dans le
    /// noyau.
    Frontiere,
    /// Parquee dans une attente interruptible : `Bloque -> Zombie` par CAS.
    TueSurPlace,
    /// Bloquee mais encore sur son coeur : `Bloque -> Pret` par CAS et
    /// publication, pour qu'elle revoie sa condamnation.
    Reveille,
    /// Parquee dans une attente non interruptible : l'evenement attendu la
    /// rendra a sa frontiere.
    AttendSonEvenement,
}

pub const fn action_tueur(v: VueTueur) -> ActionTueur {
    match v.etat {
        Etat::Zombie => ActionTueur::Rien,
        Etat::Pret => ActionTueur::Frontiere,
        Etat::Bloque if v.sur_coeur => ActionTueur::Reveille,
        Etat::Bloque if v.interruptible => ActionTueur::TueSurPlace,
        Etat::Bloque => ActionTueur::AttendSonEvenement,
    }
}

/// Apres avoir publie `Bloque`, la tache relit sa condamnation : faut-il
/// mourir sur place au lieu de dormir ?
pub const fn meurt_au_parking(condamnee: bool, interruptible: bool) -> bool {
    condamnee && interruptible
}
