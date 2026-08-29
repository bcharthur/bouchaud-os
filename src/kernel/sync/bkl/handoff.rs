// BOUCHAUD_BKL_WAITER_HANDOFF_V10
//
// Façade du handoff explicite entre une libération du BKL et le waiter
// ordinaire choisi. Tous les fragments sont `include!` dans le même module
// Rust `bkl` : aucune nouvelle frontière de visibilité ni aucun changement ABI.
//
// Ordre : état -> sélection/lease -> acquisition -> libération/réveil -> logs.

include!("handoff/etat.rs");
include!("handoff/selection.rs");
include!("handoff/acquisition.rs");
include!("handoff/release.rs");
include!("handoff/diagnostic.rs");
