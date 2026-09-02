//! Verrou noyau global reentrant pour le premier scheduler SMP de Bouchaud OS.
//!
//! Les structures historiques du noyau (`Rc<RefCell<_>>`, allocateur, RAMFS,
//! pilotes) ont ete ecrites avec l'invariant UP. Le passage immediat a des
//! verrous fins partout serait un chantier beaucoup plus vaste que le scheduler
//! lui-meme. Cette couche fournit donc un Big Kernel Lock reentrant par CPU :
//! plusieurs coeurs executent du ring 3 en parallele, mais un seul manipule les
//! structures noyau globales a la fois.
//!
//! Le scheduler peut relacher temporairement le verrou autour d'un changement de
//! contexte. Le nombre de prises reentrantes est restaure lorsque la tache reprend.
//!
//! IMPORTANT SMP/IRQ : OWNER et DEPTH sont encodes dans un meme mot atomique.
//! Aucun observateur ne peut donc voir `OWNER=local` avec une profondeur nulle.
//! Les IRQ restent masquees pendant les tres courtes transitions locales afin
//! de garder les metriques et la discipline de migration coherentes. Elles ne
//! le sont jamais pendant l'attente d'un proprietaire distant.

use core::hint::spin_loop;
use core::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use crate::kernel::sync::bkl_compte::Comptes;
use x86_64::instructions::interrupts;


//
// BOUCHAUD_BKL_FRAGMENTATION_V1
//
// Ce fichier est volontairement une facade mince. Les fragments ci-dessous
// sont inclus dans CE MEME module Rust, au lieu d'etre des `mod` Rust separes.
// C'est intentionnel : cette refactorisation ne change ni la visibilite, ni les
// chemins publics, ni les statiques, ni l'ABI. Elle ne fait que separer
// physiquement les responsabilites pour rendre les audits et les logs lisibles.
//
// Ordre de lecture conseille :
//   etat.rs          -> invariant OWNER/DEPTH + garde IRQ locale
//   metriques.rs     -> compteurs/provenance/snapshots
//   attente.rs       -> parking et protocole de reveil perdu
//   handoff.rs       -> reservation du waiter ordinaire choisi V10
//   acquisition.rs   -> enter/try/release/KernelGuard
//   ordonnanceur.rs  -> façade du contrat suspend/switch/resume
//   enregistreur.rs  -> flight recorder des transitions fautives
//
include!("bkl/etat.rs");
include!("bkl/metriques.rs");
include!("bkl/ordonnanceur.rs");
include!("bkl/attente.rs");
include!("bkl/handoff.rs");
include!("bkl/diagnostic.rs");
include!("bkl/acquisition.rs");
include!("bkl/enregistreur.rs");
