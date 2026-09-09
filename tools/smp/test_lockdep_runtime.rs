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
        pub mod cpu {
            use std::sync::atomic::{AtomicBool, Ordering};
            static ACTIVES: AtomicBool = AtomicBool::new(true);
            pub fn interrupts_enabled() -> bool { ACTIVES.load(Ordering::Acquire) }
            /// Pilote l'etat des interruptions vu par le lockdep, pour prouver
            /// que la detention IRQ-off est comptee a part.
            pub fn pose_pour_preuve(actives: bool) { ACTIVES.store(actives, Ordering::Release) }
        }
    }
}

// L'horloge du noyau. Elle avance d'un pas fixe a chaque lecture : une preuve
// ne doit pas dependre de la vitesse de la machine qui l'execute.
pub mod kernel {
    pub mod timer {
        use std::sync::atomic::{AtomicU64, Ordering};
        static HORLOGE_NS: AtomicU64 = AtomicU64::new(1_000);
        pub fn monotonic_ns() -> u64 { HORLOGE_NS.fetch_add(1_000, Ordering::AcqRel) }
    }
}

#[path = "../../src/kernel/sync/lockdep.rs"]
mod lockdep;

use lockdep::{acquired, before_acquire, depth, reinitialise_pour_preuve, released, temps, LockClass};

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

// ---------------------------------------------------------------------------
// Les temps : ce qu'un lockdep doit mesurer en plus de refuser
// ---------------------------------------------------------------------------

/// La detention est mesuree, et la classe du maximum est publiee avec lui.
///
/// Un maximum sans coupable oblige a le chercher ; avec, il se lit. C'est la
/// difference entre « un verrou a ete tenu longtemps » et « c'est le VFS ».
///
/// Le maximum est CUMULE sur toute la vie du noyau -- et donc, ici, sur tout le
/// binaire de test. Comparer un avant et un apres ne prouverait donc rien si un
/// autre cas avait deja pose un maximum plus grand : c'est ce qui a fait
/// echouer la premiere version de ce test, et le defaut etait dans le test.
///
/// Le cas force donc SON hold a etre le plus long, en consommant l'horloge de
/// preuve pendant qu'il tient `Vfs` -- par des prises de rang superieur, donc
/// licites. Aucune detention imbriquee ne peut alors l'egaler.
#[test]
fn la_detention_est_mesuree_et_attribuee() {
    reinitialise_pour_preuve();
    prend(LockClass::Vfs); // 50
    for _ in 0..64 {
        prend(LockClass::PageCache); // 60 -- rang superieur, ordre respecte
        released(LockClass::PageCache);
    }
    released(LockClass::Vfs);

    let (_, detention, classe, _) = temps();
    assert!(detention > 0, "la detention n'est pas mesuree du tout");
    assert_eq!(
        classe,
        LockClass::Vfs.rank(),
        "le maximum ne nomme pas la classe qui l'a produit"
    );
}

/// La detention INTERRUPTIONS MASQUEES est comptee a part.
///
/// C'est la mesure qui compte pour la reactivite : pendant ce temps, le coeur
/// ne prend ni tick, ni entree, ni achevement. Une detention ordinaire ralentit
/// ceux qui attendent le verrou ; une detention IRQ-off ralentit tout ce que le
/// coeur aurait du servir.
#[test]
fn la_detention_irq_off_est_comptee_a_part() {
    reinitialise_pour_preuve();
    let (_, _, _, avant) = temps();

    // Interruptions ACTIVES : ne doit rien ajouter au compteur IRQ-off.
    arch::x86_64::cpu::pose_pour_preuve(true);
    prend(LockClass::Process);
    released(LockClass::Process);
    let (_, _, _, apres_actives) = temps();
    assert_eq!(apres_actives, avant, "une detention IRQ actives a ete comptee IRQ-off");

    // Interruptions MASQUEES : doit compter.
    arch::x86_64::cpu::pose_pour_preuve(false);
    prend(LockClass::Driver);
    released(LockClass::Driver);
    let (_, _, _, apres_masquees) = temps();
    arch::x86_64::cpu::pose_pour_preuve(true);
    assert!(apres_masquees > avant, "la detention IRQ-off n'a pas ete comptee");
}

/// L'attente est mesuree meme quand aucun verrou n'est deja tenu -- c'est
/// justement le cas qui peut attendre le plus longtemps, et le manquer viderait
/// la mesure de son cas le plus interessant.
#[test]
fn l_attente_est_mesuree_des_le_premier_verrou() {
    reinitialise_pour_preuve();
    let (avant, _, _, _) = temps();
    before_acquire(LockClass::Vm);
    acquired(LockClass::Vm);
    let (apres, _, _, _) = temps();
    released(LockClass::Vm);
    assert!(apres > avant, "l'attente du premier verrou n'est pas mesuree");
}
