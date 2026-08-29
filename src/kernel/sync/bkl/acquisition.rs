// BOUCHAUD_BKL_ACQUISITION_FRAGMENTATION_V11A
//
// Façade historique conservée. Tous les fragments sont `include!` dans le
// même module `bkl` pour préserver exactement la visibilité et les statiques.
//
// Ordre de lecture : diagnostic -> garde -> libération -> enter -> try -> état.

include!("acquisition/diagnostic.rs");
include!("acquisition/guard.rs");
include!("acquisition/release.rs");
include!("acquisition/enter.rs");
include!("acquisition/try.rs");
include!("acquisition/etat.rs");
