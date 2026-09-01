// BOUCHAUD_SMP4_DEADLOCK_FIX
//! Taches utilisateur : threads, changement de contexte, futex.
//!
//! Un **processus** ([`Process`]) possede un espace d'adressage, une table de
//! descripteurs, un `brk` et une zone `mmap`. Une **tache** ([`Task`]) est un
//! fil d'execution : c'est l'unite ordonnancee. `clone(CLONE_THREAD)` cree une
//! tache de plus dans le meme processus, exactement comme sous Linux — c'est ce
//! dont `pthread_create` (donc Qt, donc Python) a besoin.
//!
//! ## Deux piles par tache
//!
//! - la **pile utilisateur**, dans l'espace d'adressage du processus ;
//! - la **pile noyau**, privee, sur laquelle s'executent ses appels systeme.
//!   C'est elle qui rend le blocage possible : quand une tache s'endort dans un
//!   `futex`, son etat noyau reste sur sa propre pile pendant qu'une autre tache
//!   utilise la sienne.
//!
//! ## Ou l'ordonnanceur peut-il commuter ?
//!
//! - a un point de blocage volontaire (`futex`, `nanosleep`, `sched_yield`,
//!   lecture bloquante) ;
//! - sur IRQ0 **uniquement si le timer a interrompu du code ring 3**
//!   ([`preempt_from_irq`]).
//!
//! Le noyau lui-meme n'est jamais preempte : il n'est pas reentrant (son
//! allocateur et ses pilotes prennent des verrous tournants), et le preempter
//! provoquerait des interblocages sur un CPU unique. Une tache utilisateur, en
//! revanche, ne detient aucun verrou noyau : la preempter est sans risque.
//!
//! ## Ce qu'une commutation doit emporter
//!
//! Ces deux chemins n'arrivent pas dans le meme etat de processeur : le premier
//! interruptions actives, le second interruptions coupees par la porte d'IRQ.
//! RFLAGS fait donc partie du contexte a sauvegarder au meme titre que les
//! registres callee-saved — voir [`switch_context`], qui explique ce que coutait
//! son oubli. L'invariant qui en decoule, verifie par [`schedule`] en
//! compilation de debogage : **on ne commute jamais interruptions coupees**, et
//! toute attente passe par [`cpu::wait_for_interrupt`] plutot que par un `hlt`
//! nu.

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use crate::arch::x86_64::{cpu, smp};
use x86_64::instructions::interrupts;
use crate::arch::x86_64::usermode::{self, TrapFrame};
use crate::kernel::fd::FdTable;
use crate::kernel::smp_lock;
use crate::kernel::vmm::AddressSpace;
use crate::kernel::sync::{SpinLock, SpinLockGuard, SpinLockIrq};
use crate::kernel::sync::{RankedSpinLock, RankedSpinLockGuard};
use crate::kernel::sync::lockdep::LockClass;
pub use crate::kernel::vma::{Backing as PromesseBacking, Vma as Promesse};

// BOUCHAUD_FINAL_V11C_DEEP_FRAGMENTATION
//
// Dernière vague V11 : `thread.rs` devient une façade mince.
// Les fragments restent dans CE module via include! : API, statiques privées,
// visibilité et ordre lexical sont conservés.

include!("thread/modeles.rs");
include!("thread/faute_memoire.rs");
include!("thread/processus.rs");
include!("thread/registre.rs");
include!("thread/tache.rs");
include!("thread/etat_global.rs");
include!("thread/diagnostic_stall.rs");
include!("thread/courant.rs");
include!("thread/creation.rs");
include!("thread/commutation.rs");
include!("thread/comptabilite.rs");
include!("thread/ordonnancement.rs");
include!("thread/lifecycle.rs");
include!("thread/blocage.rs");
include!("thread/preemption.rs");
include!("thread/metriques.rs");
include!("thread/sommeil.rs");
include!("thread/futex.rs");
include!("thread/diagnostic.rs");
