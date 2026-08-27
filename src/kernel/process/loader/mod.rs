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
//! `format` et `pe` sont complets pour ce qu'ils annoncent : reconnaitre,
//! classer, et desormais PREPARER -- `pe::prepare` rend une
//! [`image::ImagePreparee`], description projetable et neutre vis-a-vis du
//! format.
//!
//! Ce qui n'existe PAS encore : la projection elle-meme. Aucune image PE n'a
//! ete mappee ni executee par ce noyau. Tant que ce n'est pas le cas, il n'y a
//! pas de « support PE » : il y a un analyseur et un preparateur, tous deux
//! testes sur l'hote.
//!
//! ELF continue d'etre charge par `kernel::elf`, inchange -- c'est ce qui fait
//! tourner Ladybird, et le deplacer sans besoin serait un risque pour rien.

pub mod format;
pub mod image;
pub mod pe;

pub use format::{identifie, Format};
