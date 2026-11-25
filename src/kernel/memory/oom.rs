//! BOUCHAUD_OOM_CONTROLE_V1 -- quand la memoire manque, UN processus meurt,
//! pas celui qui avait la malchance de fauter.
//!
//! # Le defaut
//!
//! `memory_pressure` reclamait le cache propre, puis comptait un « evenement
//! OOM ». Rien d'autre. Une faute de page anonyme dont l'allocation echouait
//! rendait `IoError`, et le processus FAUTIF mourait de `SIGSEGV`
//! (`issue=erreur_es`) -- qu'il soit le gouffre ou le bureau qui demandait sa
//! page suivante. La machine choisissait sa victime au hasard du dernier
//! allocateur.
//!
//! # La regle
//!
//! 1. Reclamer d'abord (caches par coeur, cache de pages propres) : si une
//!    page revient, on reessaie.
//! 2. Une victime deja designee et encore vivante, tuee il y a moins de
//!    `DELAI_MORT_NS` : on l'attend -- sa memoire revient a sa mort
//!    (BOUCHAUD_ESPACE_RENDU_A_LA_MORT_V1), pas a sa recolte.
//! 3. Sinon on designe le processus vivant qui tient le plus de pages
//!    residentes, hors processus proteges et hors fils vfork (tuer un
//!    emprunteur ne rend rien). SIGKILL, puis attente.
//! 4. Si la victime designee est le fautif lui-meme, ou si personne n'est
//!    designable : on abandonne, comme avant (le fautif meurt).
//!
//! Ce module est PUR : ni `use crate::`, ni horloge, ni verrou. Le noyau lui
//! donne l'etat, il rend une decision ; `tools/memoire/test_oom.rs` le
//! contredit sur l'hote.

/// Combien de temps on attend la mort d'une victime avant d'en designer une
/// autre. Une victime prise dans une attente non interruptible (disque) peut
/// mettre du temps ; au-dela, la machine ne peut plus attendre.
pub const DELAI_MORT_NS: u64 = 3_000_000_000;

/// Ce que la faute doit faire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// De la memoire est revenue : refaire la faute.
    Reessaie,
    /// Une victime meurt : patienter un peu, puis refaire la faute.
    AttendsVictime,
    /// Designer une victime.
    ChoisisVictime,
}

/// La victime en cours, telle que le noyau la connait.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VictimeEnCours {
    pub pid: u32,
    pub tuee_a_ns: u64,
    pub vivante: bool,
}

/// L'etape suivante, apres une reclamation qui a rendu `recuperees` pages.
pub fn action(recuperees: usize, victime: Option<VictimeEnCours>, maintenant_ns: u64) -> Action {
    if recuperees > 0 {
        return Action::Reessaie;
    }
    match victime {
        Some(v) if v.vivante && maintenant_ns.saturating_sub(v.tuee_a_ns) < DELAI_MORT_NS => {
            Action::AttendsVictime
        }
        _ => Action::ChoisisVictime,
    }
}

/// Un processus que l'on pourrait tuer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Candidat {
    pub pid: u32,
    /// Pages residentes que sa mort rendrait.
    pub pages: u64,
    /// Deja mort (zombie) : sa memoire est deja rendue, ou ne le sera pas
    /// par un second signal.
    pub zombie: bool,
    /// S'execute sur le `Mm` d'un autre (fils vfork) : le tuer ne rend rien.
    pub emprunteur: bool,
    /// Protege par le systeme (bureau, superviseur...).
    pub protege: bool,
}

/// Le processus a tuer, ou aucun.
///
/// Le plus gros consommateur ; a egalite, le plus jeune (pid le plus grand),
/// qui a le moins de chances d'etre un service installe de longue date.
pub fn choisit_victime(candidats: &[Candidat]) -> Option<u32> {
    candidats
        .iter()
        .filter(|c| !c.zombie && !c.emprunteur && !c.protege && c.pages > 0)
        .max_by_key(|c| (c.pages, c.pid))
        .map(|c| c.pid)
}

/// La decision finale pour la faute de `fautif`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Tuer `pid`, puis attendre et reessayer.
    Tue(u32),
    /// Le fautif est lui-meme le plus gros, ou personne n'est designable :
    /// la faute echoue (le fautif meurt, comme avant).
    Abandonne,
}

pub fn verdict(fautif: u32, candidats: &[Candidat]) -> Verdict {
    match choisit_victime(candidats) {
        Some(pid) if pid != fautif => Verdict::Tue(pid),
        _ => Verdict::Abandonne,
    }
}
