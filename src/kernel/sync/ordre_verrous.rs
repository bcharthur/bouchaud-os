//! Un ordre unique de prise des verrous, et le moyen de le verifier.
//!
//! # Le probleme
//!
//! Deux verrous, deux chemins, deux ordres :
//!
//! ```text
//!     CPU A : lock CACHE          puis  attend entry.state
//!     CPU B : lock entry.state    puis  attend CACHE
//! ```
//!
//! Les deux CPU attendent l'autre pour toujours. Rien ne le signale : aucune
//! assertion ne se declenche, aucun compteur ne bouge, et le noyau a
//! simplement cesse d'exister.
//!
//! Un interblocage de ce genre ne se trouve pas par la mesure — il faut que
//! l'entrelacement se produise, et il peut ne jamais se produire sur la
//! machine ou l'on cherche. Il se trouve par la REGLE.
//!
//! # La regle
//!
//! Chaque verrou porte un rang. On ne peut prendre un verrou que si son rang
//! est STRICTEMENT SUPERIEUR au plus haut rang deja tenu. Un ordre total sur
//! les rangs interdit tout cycle, donc tout interblocage par inversion.
//!
//! C'est volontairement plus strict que necessaire : prendre deux verrous de
//! meme rang est refuse, meme si ce sont deux objets differents. Un cache qui
//! tiendrait deux `Entry::state` a la fois serait un cycle en puissance des
//! qu'un autre chemin les prend dans l'autre sens.
//!
//! Module pur : il ne verrouille rien. Il verifie une trace.

use alloc::vec::Vec;

/// Rang d'un verrou. Plus petit = pris plus tot.
///
/// Les rangs sont ceux du cache de pages propres. Ajouter un verrou, c'est lui
/// donner un rang ici, ce qui oblige a decider ou il se place.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Verrou {
    /// L'index du cache et sa file de candidats.
    Cache = 0,
    /// L'etat d'une entree : `Loading`, `Present`, `Failed`.
    EtatEntree = 1,
}

impl Verrou {
    pub fn nom(self) -> &'static str {
        match self {
            Verrou::Cache => "CACHE",
            Verrou::EtatEntree => "Entry::state",
        }
    }

    fn rang(self) -> u8 {
        self as u8
    }
}

/// Un evenement de la trace d'un chemin.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Evenement {
    Prend(Verrou),
    Rend(Verrou),
}

/// Ce qu'une trace peut violer.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Faute {
    /// Un verrou pris alors qu'un verrou de rang superieur ou egal est tenu.
    Inversion { pris: Verrou, deja_tenu: Verrou },
    /// Une liberation d'un verrou non tenu.
    RendSansPrendre { verrou: Verrou },
    /// Le chemin se termine en tenant encore un verrou.
    FinitEnTenant { verrou: Verrou },
}

/// Verifie une trace. `Ok(())` si l'ordre est respecte.
pub fn verifie(trace: &[Evenement]) -> Result<(), Faute> {
    let mut tenus: Vec<Verrou> = Vec::new();

    for evenement in trace.iter().copied() {
        match evenement {
            Evenement::Prend(verrou) => {
                if let Some(&plus_haut) = tenus.iter().max_by_key(|v| v.rang()) {
                    if plus_haut.rang() >= verrou.rang() {
                        return Err(Faute::Inversion { pris: verrou, deja_tenu: plus_haut });
                    }
                }
                tenus.push(verrou);
            }
            Evenement::Rend(verrou) => match tenus.iter().rposition(|&v| v == verrou) {
                Some(index) => {
                    tenus.remove(index);
                }
                None => return Err(Faute::RendSansPrendre { verrou }),
            },
        }
    }

    match tenus.first() {
        Some(&verrou) => Err(Faute::FinitEnTenant { verrou }),
        None => Ok(()),
    }
}
