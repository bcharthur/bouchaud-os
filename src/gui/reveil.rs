//! Comptabilite du reveil evenementiel du compositeur.
//!
//! Le mecanisme lui-meme vit dans [`crate::kernel::sync::reveil`] : c'est une
//! primitive noyau, et les producteurs -- pilote PS/2, couche descripteurs --
//! n'ont aucune raison de connaitre le bureau. Ce module-ci ne fait que deux
//! choses que le noyau n'a pas a faire :
//!
//!   * compter ce que le COMPOSITEUR, lui, a decide d'en faire -- combien de
//!     tours, combien de trames composees, combien sautees ;
//!   * les publier sous une forme lisible.
//!
//! # Pourquoi separer trames « horloge » et trames « utiles »
//!
//! L'horloge de la barre des taches change toute seule une fois par seconde, et
//! rien ne peut l'annoncer (voir `politique::PERIODE_HORLOGE_MS`). Un bureau
//! parfaitement immobile compose donc une trame par seconde, pour toujours.
//!
//! Melanger ces trames-la aux autres rendrait la mesure d'inactivite
//! inutilisable : on ne saurait jamais si les trente trames d'un repos de
//! trente secondes viennent de l'horloge -- normal -- ou d'un reveil parasite
//! -- une panne. Elles sont donc comptees a part.

use core::sync::atomic::{AtomicU64, Ordering};

use crate::kernel::sync::reveil::{Source, INTERFACE, NOMBRE_SOURCES, NOMS_SOURCES};

/// Tours de boucle du compositeur.
static TOURS: AtomicU64 = AtomicU64::new(0);
/// Trames composees parce qu'un degat le demandait.
static TRAMES_COMPOSEES: AtomicU64 = AtomicU64::new(0);
/// Trames dont seule l'horloge est responsable.
static TRAMES_HORLOGE: AtomicU64 = AtomicU64::new(0);
/// Tours ou l'on avait un degat mais pas encore le creneau de trame.
static TRAMES_DIFFEREES: AtomicU64 = AtomicU64::new(0);
/// Recompositions « a l'aveugle » d'un client muet.
static RECOMPOSITIONS_AVEUGLES: AtomicU64 = AtomicU64::new(0);
/// Sommeils sans echeance : le bureau n'avait strictement aucune raison de se
/// reveiller. C'est la mesure que Gate 1B cherche a faire monter.
static SOMMEILS_SANS_FIN: AtomicU64 = AtomicU64::new(0);

#[inline]
pub fn note_tour() {
    TOURS.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn note_trame(horloge_seule: bool) {
    TRAMES_COMPOSEES.fetch_add(1, Ordering::Relaxed);
    if horloge_seule {
        TRAMES_HORLOGE.fetch_add(1, Ordering::Relaxed);
    }
}

#[inline]
pub fn note_trame_differee() {
    TRAMES_DIFFEREES.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn note_recomposition_aveugle() {
    RECOMPOSITIONS_AVEUGLES.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn note_sommeil_sans_fin() {
    SOMMEILS_SANS_FIN.fetch_add(1, Ordering::Relaxed);
}

/// Signale du travail au compositeur depuis le bureau lui-meme.
///
/// Les producteurs externes passent par `kernel::sync::signale_interface` ; ce
/// raccourci sert aux changements decides DANS la boucle (fenetre ouverte,
/// deplacee, fermee) qui doivent survivre a un sommeil declenche juste apres.
#[inline]
pub fn invalide(source: Source) {
    INTERFACE.signale(source);
}

/// Publie la ligne `[GUI-COMPOSITOR]`. Une par releve, jamais une par trame.
pub fn publie() {
    let (sommeils, evites, reveils_signal, reveils_echeance) = INTERFACE.statistiques();
    let composees = TRAMES_COMPOSEES.load(Ordering::Relaxed);
    let horloge = TRAMES_HORLOGE.load(Ordering::Relaxed);

    crate::serial_println!(
        "[GUI-COMPOSITOR] wakeups={} invalidations={} frames_requested={} \
         frames_composed={} frames_clock_only={} frames_useful={} frames_skipped={} \
         blind_recomposes={} idle_sleeps={} idle_sleeps_avoided={} \
         idle_wakeups_signal={} idle_wakeups_deadline={} sleeps_unbounded={} loops={}",
        reveils_signal.saturating_add(reveils_echeance),
        INTERFACE.invalidations_totales(),
        composees.saturating_add(TRAMES_DIFFEREES.load(Ordering::Relaxed)),
        composees,
        horloge,
        composees.saturating_sub(horloge),
        TRAMES_DIFFEREES.load(Ordering::Relaxed),
        RECOMPOSITIONS_AVEUGLES.load(Ordering::Relaxed),
        sommeils,
        evites,
        reveils_signal,
        reveils_echeance,
        SOMMEILS_SANS_FIN.load(Ordering::Relaxed),
        TOURS.load(Ordering::Relaxed),
    );

    // Le detail par source : « 4000 reveils » ne dit pas s'il faut regarder la
    // souris, un client bavard ou une echeance mal choisie.
    let mut ligne = alloc::string::String::from("[GUI-COMPOSITOR-SOURCES]");
    for index in 0..NOMBRE_SOURCES {
        let source = match index {
            0 => Source::Clavier,
            1 => Source::Souris,
            2 => Source::Client,
            3 => Source::Fenetre,
            _ => Source::Explicite,
        };
        ligne.push_str(&alloc::format!(
            " {}={}",
            NOMS_SOURCES[index],
            INTERFACE.invalidations(source),
        ));
    }
    crate::serial_println!("{}", ligne);
}
