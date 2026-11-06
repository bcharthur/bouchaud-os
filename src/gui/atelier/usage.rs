//! M12 : modes d'usage purs, sans politique systeme simulee.
//! Cette tranche n'agit que sur mouvement et diagnostic. Ni priorite CPU,
//! ni politique reseau/energie ou service d'arriere-plan ne change ici.
use super::animations::Mode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Usage { Standard, Travail, Focus, Jeu, Personnalise }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReglagesVisuels { pub animations: Mode, pub diagnostic: bool }

pub const fn profil(usage: Usage) -> Option<ReglagesVisuels> {
    match usage {
        Usage::Standard => Some(ReglagesVisuels { animations: Mode::Normal, diagnostic: false }),
        Usage::Travail => Some(ReglagesVisuels { animations: Mode::Reduit, diagnostic: false }),
        Usage::Focus => Some(ReglagesVisuels { animations: Mode::Aucun, diagnostic: false }),
        Usage::Jeu => Some(ReglagesVisuels { animations: Mode::Normal, diagnostic: true }),
        Usage::Personnalise => None,
    }
}
pub fn depuis_env(v: &str) -> Option<Usage> {
    match v.trim() {
        "standard" => Some(Usage::Standard),
        "travail" | "work" => Some(Usage::Travail),
        "focus" => Some(Usage::Focus),
        "jeu" | "gaming" => Some(Usage::Jeu),
        _ => None,
    }
}
