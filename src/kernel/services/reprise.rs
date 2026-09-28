//! Politique pure, sans horloge implicite ni allocation, pour les seuls fils
//! noyau dont l'entree peut etre relancee. Pas de relance des processus arbitraires.
// P18_SERVICE_GUARDIAN_V1
pub const N: usize = 6;
pub const FENETRE_MS: u64 = 300_000;
pub const MAX_ESSAIS: u8 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fil { Mesures, Audit, Telemetrie, Brdp, Reception, Lien }
impl Fil {
    pub const TOUS: [Self; N] = [Self::Mesures, Self::Audit, Self::Telemetrie,
                                  Self::Brdp, Self::Reception, Self::Lien];
    pub const fn index(self) -> usize { self as usize }
    pub const fn nom(self) -> &'static str { match self {
        Self::Mesures => "services-metrics", Self::Audit => "bouchaud-auditd",
        Self::Telemetrie => "bouchaud-telemetrie", Self::Brdp => "bouchaud-brdp",
        Self::Reception => "net-rx-poll", Self::Lien => "net-lien",
    } }
    pub fn depuis_nom(nom: &str) -> Option<Self> {
        Self::TOUS.iter().copied().find(|fil| fil.nom() == nom)
    }
}
#[derive(Clone, Copy)]
struct Etat { en_attente: bool, depuis_ms: u64, prochaine_ms: u64, essais: u8, en_cours: bool }
const VIDE: Etat = Etat { en_attente: false, depuis_ms: 0, prochaine_ms: 0,
                          essais: 0, en_cours: false };
pub struct Politique { etats: [Etat; N], morts: u64, relances: u64, echecs: u64, refuses: u64 }
impl Politique {
    pub const fn nouvelle() -> Self {
        Self { etats: [VIDE; N], morts: 0, relances: 0, echecs: 0, refuses: 0 }
    }
    pub fn mort(&mut self, fil: Fil, maintenant_ms: u64) {
        let s = &mut self.etats[fil.index()];
        if !s.en_attente {
            if maintenant_ms.saturating_sub(s.depuis_ms) > FENETRE_MS {
                s.essais = 0;
                s.depuis_ms = maintenant_ms;
            }
            s.en_attente = true;
            s.en_cours = false;
            s.prochaine_ms = maintenant_ms.saturating_add(2_000);
            self.morts = self.morts.saturating_add(1);
        }
    }
    /// Reserve un essai sous le verrou; l'appel au lanceur se fait hors verrou.
    pub fn reserve(&mut self, maintenant_ms: u64) -> Option<(Fil, u8)> {
        for fil in Fil::TOUS {
            let s = &mut self.etats[fil.index()];
            if !s.en_attente || s.en_cours || maintenant_ms < s.prochaine_ms { continue; }
            if maintenant_ms.saturating_sub(s.depuis_ms) > FENETRE_MS {
                s.depuis_ms = maintenant_ms;
                s.essais = 0;
            }
            if s.essais >= MAX_ESSAIS {
                // Un seul refus par fenetre; la reprise reste possible plus tard.
                s.prochaine_ms = s.depuis_ms.saturating_add(FENETRE_MS + 1);
                self.refuses = self.refuses.saturating_add(1);
                continue;
            }
            s.essais += 1;
            s.en_cours = true;
            return Some((fil, s.essais));
        }
        None
    }
    pub fn resultat(&mut self, fil: Fil, maintenant_ms: u64, lance: bool) {
        let s = &mut self.etats[fil.index()];
        if !s.en_cours { return; }
        s.en_cours = false;
        if lance {
            s.en_attente = false;
            self.relances = self.relances.saturating_add(1);
        } else {
            s.prochaine_ms = maintenant_ms.saturating_add(2_000u64 << s.essais.min(3));
            self.echecs = self.echecs.saturating_add(1);
        }
    }
    pub fn compteurs(&self) -> (u64, u64, u64, u64) {
        (self.morts, self.relances, self.echecs, self.refuses)
    }
}
