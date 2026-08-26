//! Frontiere entre le FORMAT d'un executable et le PROCESSUS qui l'accueille.
//!
//! # Pourquoi cette frontiere
//!
//! `exec` melangeait les deux : il connaissait ELF, ses en-tetes de programme
//! et sa pile, en meme temps que l'espace d'adressage et la table des
//! descripteurs. Un second format n'avait aucun endroit ou exister.
//!
//! La separation prepare aussi la reduction du gros verrou dans `execve`, que
//! le journal montre tenu des centaines de millisecondes : tout ce qui est ici
//! est du travail de LECTURE -- reconnaitre, valider, decider -- et n'a besoin
//! d'aucun etat partage. C'est exactement la part qui pourra sortir du verrou
//! quand `prepare_exec` / `commit_exec` seront separes.
//!
//! # Etat
//!
//! `format` et `pe` sont complets pour ce qu'ils annoncent : reconnaitre et
//! classer. ELF continue d'etre charge par `kernel::elf`, inchange -- c'est ce
//! qui fait tourner Ladybird, et le deplacer sans besoin serait un risque pour
//! rien. Le chargement d'un PE Bouchaud n'existe pas encore.

pub mod format;
pub mod pe;

pub use format::{identifie, Format};
