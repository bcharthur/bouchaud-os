//! Ce qu'un chemin noyau a le droit de faire pendant qu'il tient le gros verrou.
//!
//! # Pourquoi une regle et pas une mesure
//!
//! Le runtime a montre un BKL detenu 99,96 % d'une fenetre pour 77 acquisitions
//! seulement : des TENUES LONGUES, pas une frequence excessive. Chercher cela
//! par la mesure suppose de reproduire la charge ; le trouver par la regle ne
//! le suppose pas.
//!
//! Les trois defauts reels avaient la meme forme, et aucune mesure ne l'aurait
//! nommee : le cout d'une operation faite sous le verrou dependait de la taille
//! d'une STRUCTURE GLOBALE, pas de celle de la demande.
//!
//!   * `free_frame` parcourait la liste libre entiere — dont la longueur ne
//!     depend pas de ce qu'on libere, mais de l'age de la session ;
//!   * `prepare_unmap` et `finish_unmap` balayaient toutes les frames
//!     residentes du processus — pour une plage de quelques pages ;
//!   * `release` du cache de pages propres recomptait tout le cache — a chaque
//!     page rendue.
//!
//! D'ou la regle centrale de ce module :
//!
//! > Sous le gros verrou, le cout d'une phase peut dependre de la TAILLE DE LA
//! > DEMANDE. Il ne doit jamais dependre de la taille d'un ETAT GLOBAL.
//!
//! Une plage de mille pages coute mille fois une page : c'est le travail
//! demande, et l'appelant l'a demande. Une plage d'une page qui coute cent
//! mille comparaisons parce que la machine tourne depuis longtemps, non.
//!
//! # Les deux autres regles
//!
//! Dormir en tenant le verrou bloque tous les autres CPU pendant un temps que
//! rien ne borne. Et un chemin qui reprend le verrou apres un changement de
//! contexte doit le rendre avant de bloquer de nouveau, sinon il le tient
//! pendant une attente.
//!
//! Module pur : il ne verrouille rien et ne mesure rien. Il verifie une trace.

use alloc::vec::Vec;

/// De quoi depend le cout d'une phase.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cout {
    /// Un nombre d'operations borne, quelle que soit la demande.
    Constant,
    /// Logarithmique en la taille d'une structure : un index, un arbre.
    Logarithmique,
    /// Lineaire en la TAILLE DE LA DEMANDE — la plage traitee, le nombre de
    /// descripteurs sondes. Admis sous le verrou : c'est le travail demande.
    LineaireEnDemande,
    /// Lineaire en la taille d'un ETAT GLOBAL — liste libre, cache de pages,
    /// ensemble resident, table des taches. INTERDIT sous le verrou.
    LineaireEnEtatGlobal,
}

/// Un evenement de la trace d'un chemin noyau.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Evenement {
    /// Prise du gros verrou (`smp_lock::enter`).
    Prend,
    /// Liberation d'un niveau (`Drop` du garde).
    Rend,
    /// Liberation complete avant un changement de contexte.
    Suspend,
    /// Reprise a la profondeur d'avant (`resume_after_schedule`).
    Reprend,
    /// Une phase de travail, avec ce dont son cout depend.
    Phase(&'static str, Cout),
    /// La pile s'endort : HLT, `park_current_on`, attente d'un ACK distant.
    Dort(&'static str),
}

/// Ce qu'une trace peut violer.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Faute {
    /// Le chemin dort en tenant le verrou.
    DortSousVerrou { quoi: &'static str, profondeur: usize },
    /// Une phase dont le cout depend d'un etat global s'execute sous le verrou.
    PhaseGlobaleSousVerrou { phase: &'static str, profondeur: usize },
    /// Une liberation sans prise correspondante.
    RendSansPrendre,
    /// Une reprise alors que le verrou n'avait pas ete suspendu.
    ReprendSansSuspendre,
    /// Le chemin se termine en tenant encore le verrou qu'il a pris.
    FinitEnTenant { profondeur: usize },
    /// Le verrou a ete suspendu et jamais repris.
    SuspenduSansReprise,
}

/// Verifie une trace. `Ok(())` si le chemin respecte les trois regles.
pub fn verifie(trace: &[Evenement]) -> Result<(), Faute> {
    let mut profondeur = 0usize;
    let mut suspendues: Vec<usize> = Vec::new();

    for evenement in trace.iter().copied() {
        match evenement {
            Evenement::Prend => profondeur += 1,
            Evenement::Rend => {
                if profondeur == 0 {
                    return Err(Faute::RendSansPrendre);
                }
                profondeur -= 1;
            }
            Evenement::Suspend => {
                // Suspendre sans rien tenir est un non-evenement, comme dans
                // `suspend_for_schedule` qui rend 0.
                suspendues.push(profondeur);
                profondeur = 0;
            }
            Evenement::Reprend => match suspendues.pop() {
                Some(ancienne) => profondeur = ancienne,
                None => return Err(Faute::ReprendSansSuspendre),
            },
            Evenement::Phase(nom, cout) => {
                if profondeur > 0 && cout == Cout::LineaireEnEtatGlobal {
                    return Err(Faute::PhaseGlobaleSousVerrou { phase: nom, profondeur });
                }
            }
            Evenement::Dort(quoi) => {
                if profondeur > 0 {
                    return Err(Faute::DortSousVerrou { quoi, profondeur });
                }
            }
        }
    }

    if !suspendues.is_empty() {
        return Err(Faute::SuspenduSansReprise);
    }
    if profondeur != 0 {
        return Err(Faute::FinitEnTenant { profondeur });
    }
    Ok(())
}
