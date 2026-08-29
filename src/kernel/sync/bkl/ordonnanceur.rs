// Pont scheduler <-> BKL.
//
// Ce fichier est maintenant une façade. Les sous-fichiers restent compilés
// dans le même module Rust `bkl` via `include!`, donc l'API et la visibilité
// restent identiques à V4.
//
// Ordre : état -> priorité -> trace -> suspend -> resume.

include!("ordonnanceur/etat.rs");
include!("ordonnanceur/priorite.rs");
include!("ordonnanceur/trace.rs");
include!("ordonnanceur/suspend.rs");
include!("ordonnanceur/resume.rs");
