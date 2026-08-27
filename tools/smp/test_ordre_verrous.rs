//! L'ordre de prise des verrous du cache de pages propres.
//!
//! # Ce que ces tests verifient
//!
//! Un seul ordre est permis :
//!
//!     CACHE  ->  Entry::state
//!
//! Les traces ci-dessous decrivent les chemins REELS. Chacun est double d'une
//! version inversee, qui doit etre refusee : c'est l'interblocage SMP que
//! l'entrelacement
//!
//!     CPU A : lock CACHE       puis attend state
//!     CPU B : lock state       puis attend CACHE
//!
//! produirait, et qu'aucune mesure ne trouverait — il faut que l'entrelacement
//! se produise, et il peut ne jamais se produire sur la machine ou l'on
//! cherche.
//!
//! # Ce que ces tests ne prouvent pas
//!
//! Que les traces decrivent fidelement le code. C'est une lecture. La partie
//! mecanique est faite par `tools/verifie-ordre-verrous.py`, qui lit la source
//! et echoue si un chemin `state -> CACHE` reapparait.
//!
//! Lance par `tools/smp/test-ordre-verrous.sh`.

extern crate alloc;

#[path = "../../src/kernel/sync/ordre_verrous.rs"]
mod ordre_verrous;

use ordre_verrous::{verifie, Evenement, Faute, Verrou};
use Evenement::{Prend, Rend};
use Verrou::{Cache, EtatEntree};

// ─── Les chemins reels ─────────────────────────────────────────────────────

/// `acquire`, coup au but : `CACHE` tenu pendant la lecture de l'etat.
fn acquire_touche() -> alloc::vec::Vec<Evenement> {
    alloc::vec![Prend(Cache), Prend(EtatEntree), Rend(EtatEntree), Rend(Cache)]
}

/// `acquire`, chargement echoue : publier l'etat, PUIS proposer la cle.
///
/// C'est le chemin signale a la relecture. Le garde d'etat est desormais nomme
/// et relache explicitement avant `CACHE.lock()`.
fn acquire_chargement_echoue() -> alloc::vec::Vec<Evenement> {
    alloc::vec![
        Prend(Cache), Prend(EtatEntree), Rend(EtatEntree), Rend(Cache),  // reservation
        // ... lecture disque, aucun verrou tenu ...
        Prend(EtatEntree),          // publication de State::Failed
        Rend(EtatEntree),           // drop(etat) EXPLICITE
        Prend(Cache),               // propose(key)
        Rend(Cache),
    ]
}

/// `release` : decrementer, relacher, puis proposer.
fn release() -> alloc::vec::Vec<Evenement> {
    alloc::vec![
        Prend(Cache), Rend(Cache),          // get(&key).cloned()
        Prend(EtatEntree), Rend(EtatEntree),// decrement, drop(etat)
        Prend(Cache), Rend(Cache),          // propose(key)
    ]
}

/// `retire_un_candidat` : appele avec `CACHE` deja tenu.
fn retire_un_candidat() -> alloc::vec::Vec<Evenement> {
    alloc::vec![
        Prend(Cache),
        Prend(EtatEntree), Rend(EtatEntree), // validation du candidat
        Prend(EtatEntree), Rend(EtatEntree), // candidat suivant
        Rend(Cache),
    ]
}

/// `reclaim_excess` : `CACHE` tenu, puis l'etat de la victime.
fn reclaim_excess() -> alloc::vec::Vec<Evenement> {
    alloc::vec![
        Prend(Cache),
        Prend(EtatEntree), Rend(EtatEntree),
        Prend(EtatEntree),
        Rend(EtatEntree),
        Rend(Cache),
    ]
}

/// `lifetime_stats` : le comptage exact du releve.
fn lifetime_stats() -> alloc::vec::Vec<Evenement> {
    alloc::vec![
        Prend(Cache),
        Prend(EtatEntree), Rend(EtatEntree),
        Prend(EtatEntree), Rend(EtatEntree),
        Rend(Cache),
    ]
}

#[test]
fn tous_les_chemins_du_cache_respectent_l_ordre() {
    for (nom, trace) in [
        ("acquire (touche)", acquire_touche()),
        ("acquire (chargement echoue)", acquire_chargement_echoue()),
        ("release", release()),
        ("retire_un_candidat", retire_un_candidat()),
        ("reclaim_excess", reclaim_excess()),
        ("lifetime_stats", lifetime_stats()),
    ] {
        assert_eq!(verifie(&trace), Ok(()), "{nom}");
    }
}

// ─── L'inversion, chemin par chemin ────────────────────────────────────────

/// LE defaut signale : publier l'etat puis prendre `CACHE` SANS relacher.
#[test]
fn publier_l_etat_puis_prendre_le_cache_sans_relacher_est_refuse() {
    let trace = alloc::vec![
        Prend(EtatEntree),   // *entry.state.lock() = State::Failed
        Prend(Cache),        // CACHE.lock().propose(key)  <- inversion
        Rend(Cache),
        Rend(EtatEntree),
    ];
    assert_eq!(
        verifie(&trace),
        Err(Faute::Inversion { pris: Cache, deja_tenu: EtatEntree }),
    );
}

#[test]
fn release_qui_propose_sans_relacher_l_etat_est_refuse() {
    let trace = alloc::vec![
        Prend(Cache), Rend(Cache),
        Prend(EtatEntree),
        Prend(Cache),          // propose avant drop(etat)  <- inversion
        Rend(Cache),
        Rend(EtatEntree),
    ];
    assert!(matches!(verifie(&trace), Err(Faute::Inversion { .. })));
}

/// L'entrelacement complet, ecrit tel qu'il se produirait.
#[test]
fn l_entrelacement_qui_bloque_les_deux_cpu_est_refuse() {
    // CPU A respecte l'ordre ; CPU B ne le respecte pas. Il suffit qu'UN seul
    // chemin l'enfreigne pour que le cycle existe.
    assert_eq!(verifie(&acquire_touche()), Ok(()), "CPU A est correct");
    let cpu_b = alloc::vec![Prend(EtatEntree), Prend(Cache), Rend(Cache), Rend(EtatEntree)];
    assert!(
        verifie(&cpu_b).is_err(),
        "il suffit qu'un seul chemin inverse pour que le cycle existe"
    );
}

// ─── La regle elle-meme ────────────────────────────────────────────────────

#[test]
fn deux_verrous_de_meme_rang_sont_refuses() {
    // Tenir deux `Entry::state` a la fois : un cycle en puissance des qu'un
    // autre chemin les prend dans l'autre sens.
    let trace = alloc::vec![
        Prend(EtatEntree), Prend(EtatEntree), Rend(EtatEntree), Rend(EtatEntree),
    ];
    assert_eq!(
        verifie(&trace),
        Err(Faute::Inversion { pris: EtatEntree, deja_tenu: EtatEntree }),
    );
}

#[test]
fn prendre_l_etat_seul_est_permis() {
    assert_eq!(verifie(&[Prend(EtatEntree), Rend(EtatEntree)]), Ok(()));
}

#[test]
fn prendre_le_cache_seul_est_permis() {
    assert_eq!(verifie(&[Prend(Cache), Rend(Cache)]), Ok(()));
}

#[test]
fn relacher_dans_l_ordre_inverse_de_la_prise_est_permis() {
    let trace = alloc::vec![
        Prend(Cache), Prend(EtatEntree), Rend(EtatEntree), Rend(Cache),
    ];
    assert_eq!(verifie(&trace), Ok(()));
}

/// Relacher `CACHE` avant `state` est admis : l'ordre porte sur la PRISE.
#[test]
fn relacher_le_cache_avant_l_etat_est_permis() {
    let trace = alloc::vec![
        Prend(Cache), Prend(EtatEntree), Rend(Cache), Rend(EtatEntree),
    ];
    assert_eq!(verifie(&trace), Ok(()));
}

#[test]
fn rendre_un_verrou_non_tenu_est_refuse() {
    assert_eq!(
        verifie(&[Rend(Cache)]),
        Err(Faute::RendSansPrendre { verrou: Cache }),
    );
}

#[test]
fn finir_en_tenant_un_verrou_est_refuse() {
    assert_eq!(
        verifie(&[Prend(Cache)]),
        Err(Faute::FinitEnTenant { verrou: Cache }),
    );
}

#[test]
fn une_trace_vide_est_correcte() {
    assert_eq!(verifie(&[]), Ok(()));
}

#[test]
fn les_rangs_sont_ordonnes_et_nommes() {
    assert!(Verrou::Cache < Verrou::EtatEntree, "CACHE se prend en premier");
    assert_eq!(Verrou::Cache.nom(), "CACHE");
    assert_eq!(Verrou::EtatEntree.nom(), "Entry::state");
}
