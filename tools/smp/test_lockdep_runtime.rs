//! Preuve hote du lockdep RUNTIME.
//!
//! `test_ordre_verrous.rs` met a l'epreuve le verificateur STATIQUE : il prouve
//! des traces de modele, ecrites a la main. Ce fichier-ci met a l'epreuve
//! l'autre moitie -- celle qui tourne dans le noyau, sur les verrous reels, et
//! qui n'avait aucune preuve.
//!
//! # Ce qu'un lockdep doit faire, et ce qu'il ne prouve pas
//!
//! Il ne prouve pas l'absence d'interblocage. Il transforme une classe
//! d'interblocages en PANNE ATTRIBUABLE : au lieu que deux coeurs s'attendent
//! l'un l'autre pour toujours -- un gel sans coupable, qui n'arrive que sur
//! l'entrelacement qu'on n'a pas reproduit --, le premier coeur qui viole
//! l'ordre s'arrete en nommant la classe, le rang et le fichier.
//!
//! C'est pour cela que les cas ci-dessous VIOLENT deliberement l'invariant. Un
//! detecteur qu'on ne teste qu'avec des traces licites ne prouve rien : il
//! passerait tout aussi bien s'il ne detectait rien du tout.
//!
//! # L'entrelacement que le rang interdit
//!
//!     CPU A : prend Vfs (50)      puis attend FdTable (40)
//!     CPU B : prend FdTable (40)  puis attend Vfs (50)
//!
//! Aucun des deux ne rendra jamais. Le rang l'interdit en refusant, sur UN seul
//! coeur, la premiere moitie de l'entrelacement -- celle de A --, sans avoir
//! besoin de voir B ni de reproduire la course.

// `log_stats` publie sur COM1. L'hote n'a pas de port serie ; la macro est
// rendue muette pour que le module de PRODUCTION soit inclus tel quel, plutot
// qu'une copie qui lui ressemblerait.
macro_rules! serial_println {
    ($($arg:tt)*) => {{}};
}
pub(crate) use serial_println;

// `lockdep.rs` s'adresse au CPU courant par `crate::arch::x86_64::smp`. L'hote
// n'a pas de SMP ; un seul coeur suffit a prouver l'ordre, qui est une
// propriete LOCALE a un coeur.
pub mod arch {
    pub mod x86_64 {
        pub mod smp {
            pub const MAX_CPUS: usize = 8;
            pub fn cpu_index() -> usize { 0 }
        }
    }
}

#[path = "../../src/kernel/sync/lockdep.rs"]
mod lockdep;

use lockdep::{acquired, before_acquire, depth, reinitialise_pour_preuve, released, LockClass};

/// Prend un verrou comme le noyau le fait : controle puis enregistrement.
fn prend(classe: LockClass) {
    before_acquire(classe);
    acquired(classe);
}

// ---------------------------------------------------------------------------
// Ce qui doit passer
// ---------------------------------------------------------------------------

/// L'ordre croissant est le seul permis, et il ne coute rien.
#[test]
fn l_ordre_croissant_est_accepte() {
    reinitialise_pour_preuve();
    prend(LockClass::ProcessTable); // 20
    prend(LockClass::FdTable); //     40
    prend(LockClass::Vfs); //         50
    prend(LockClass::Vm); //          70
    assert_eq!(depth(), 4);
    released(LockClass::Vm);
    released(LockClass::Vfs);
    released(LockClass::FdTable);
    released(LockClass::ProcessTable);
    assert_eq!(depth(), 0);
}

/// Prendre et rendre en boucle ne fait pas deriver la profondeur. Une derive
/// serait invisible jusqu'a ce que la pile deborde, tres loin de sa cause.
#[test]
fn la_profondeur_ne_derive_pas() {
    reinitialise_pour_preuve();
    for _ in 0..1000 {
        prend(LockClass::Vfs);
        released(LockClass::Vfs);
    }
    assert_eq!(depth(), 0);
}

// ---------------------------------------------------------------------------
// Ce qui doit ECHOUER -- et c'est tout l'objet du detecteur
// ---------------------------------------------------------------------------

/// L'INVERSION AB/BA. Le cas pour lequel ce module existe.
///
/// Tenir `Vfs` (rang 50) puis demander `FdTable` (rang 40), c'est la moitie de
/// l'entrelacement qui interbloque deux coeurs. Le detecteur doit refuser CETTE
/// moitie, sur un seul coeur, sans avoir a reproduire la course.
#[test]
#[should_panic(expected = "LOCKDEP inversion")]
fn une_inversion_ab_ba_est_refusee() {
    reinitialise_pour_preuve();
    prend(LockClass::Vfs); // 50
    prend(LockClass::FdTable); // 40 -- descendant : interdit
}

/// Reprendre la MEME classe est aussi une inversion : le rang doit
/// STRICTEMENT croitre. Deux verrous de meme rang pris dans un ordre relatif
/// que rien ne fixe s'interbloquent exactement comme deux rangs inverses.
#[test]
#[should_panic(expected = "LOCKDEP inversion")]
fn reprendre_la_meme_classe_est_refuse() {
    reinitialise_pour_preuve();
    prend(LockClass::Process);
    prend(LockClass::Process);
}

/// Rendre dans le desordre casse l'hypothese sur laquelle toute la pile repose.
/// Si l'on tolerait un rendu non-LIFO, la pile ne decrirait plus ce qui est
/// tenu, et les inversions suivantes passeraient inapercues.
#[test]
#[should_panic(expected = "LOCKDEP non-LIFO")]
fn un_rendu_dans_le_desordre_est_refuse() {
    reinitialise_pour_preuve();
    prend(LockClass::FdTable);
    prend(LockClass::Vfs);
    released(LockClass::FdTable); // le sommet est Vfs
}

/// Rendre un verrou qu'on n'a pas pris signale un desequilibre -- typiquement
/// un garde rendu deux fois, ou une sortie de fonction qui saute un `Drop`.
#[test]
#[should_panic(expected = "LOCKDEP release without acquisition")]
fn un_rendu_sans_prise_est_refuse() {
    reinitialise_pour_preuve();
    released(LockClass::Vfs);
}
