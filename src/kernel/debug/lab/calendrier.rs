//! Le calendrier des points de reprise : une echeance ratee ne coute plus 45 s.
//!
//! # Le defaut, tel que le physique l'a ecrit
//!
//! Releve TRIGKEY, campagne `1179cdd` :
//!
//! ```text
//! session reelle     ~395 s
//! records persistes  ~193 s
//! checkpoints        0
//! fin                false
//! ```
//!
//! Deux cents secondes de session n'ont jamais atteint le support, et pas un
//! seul point de reprise n'a ete pose en pres de sept minutes. Le fil
//! `services-metrics` tournait normalement -- il n'y a pas eu de famine, et
//! sa priorite n'est pas en cause.
//!
//! L'enchainement etait celui-ci :
//!
//! ```text
//! si maintenant < prochain            -> rien
//! prochain = maintenant + 45 000 ms   <-- AVANT de savoir si le support suit
//! si support absent                   -> rien
//! poser le checkpoint
//! ```
//!
//! L'echeance etait donc CONSOMMEE par une tentative qui n'avait pas lieu. Au
//! premier passage -- cent millisecondes apres l'amorcage, bien avant qu'une
//! cle USB soit enumeree -- l'echeance passait a quarante-cinq secondes, et
//! le premier point de reprise possible tombait vers la quarante-septieme.
//! Chaque echeance ratee ensuite coutait quarante-cinq secondes de plus.
//!
//! # Ce que ce module change
//!
//! Une echeance ne se consomme que lorsqu'une tentative a REELLEMENT eu lieu.
//! Sans support, on reporte de peu -- une seconde, puis deux -- et l'on pose
//! des que le support repond :
//!
//! ```text
//! support absent a t=0, pret a t=5    ->  checkpoint vers t=5..7
//! (avant : t=45..47)
//! ```
//!
//! # Ce module ne touche a rien
//!
//! Pas de support, pas d'horloge propre, pas d'etat global. La boite noire
//! lui donne l'heure et l'etat du support ; il rend une decision. La regle se
//! contredit donc en test hote, sans cle USB et sans machine.

/// Cadence de croisiere, en millisecondes.
///
/// Valeur de depart, a confirmer par la mesure du cout reel. Elle vient de
/// `blackbox::PERIODE_CHECKPOINT_MS`, dont ce module reprend la politique.
pub const PERIODE_MS: u64 = 45_000;

/// Report quand le support n'est pas encore la, premiere fois.
pub const REPORT_COURT_MS: u64 = 1_000;

/// Report des fois suivantes.
///
/// # Pourquoi il ne croit PAS jusqu'a la periode normale
///
/// Un report qui doublerait jusqu'a quarante-cinq secondes ramenerait
/// exactement le defaut qu'on corrige, en plus lent : une cle branchee a la
/// trentieme seconde attendrait de nouveau la soixante-quinzieme. Deux
/// secondes de palier, c'est une lecture d'atomique toutes les deux secondes
/// sur une machine sans cle -- un cout qu'on peut payer indefiniment.
pub const REPORT_PALIER_MS: u64 = 2_000;

/// Reports courts accordes a un checkpoint qui ECHOUE, support present.
///
/// Un support qui repond mais n'ecrit pas est peut-etre en train de mourir.
/// On lui laisse trois chances rapprochees, puis on retombe a la cadence
/// normale : marteler une cle defaillante ne la repare pas, et le fil qui
/// porte la persistance a autre chose a faire.
pub const ECHECS_AVANT_CADENCE_NORMALE: u32 = 3;

/// Pourquoi une echeance a ete reportee. Chiffre, pour tenir dans l'anneau.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u32)]
pub enum Raison {
    /// Le support n'etait pas pret.
    SansSupport = 1,
    /// Le support etait pret, et le checkpoint a echoue.
    EchecEcriture = 2,
}

/// Ce que le calendrier repond quand on le consulte.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Decision {
    /// L'echeance n'est pas atteinte.
    Attendre { restant_ms: u64 },
    /// Echeance atteinte et support pret : posez, puis dites ce que ca a donne.
    Pose,
    /// Echeance atteinte, support absent. L'echeance n'est PAS consommee.
    Reporte { raison: Raison, prochain_ms: u64 },
}

/// L'etat du calendrier. Pur, copiable, sans dependance.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Calendrier {
    prochain_ms: u64,
    reports_consecutifs: u32,
    echecs_consecutifs: u32,
    poses: u64,
    dernier_ok_ms: u64,
    dernier_essai_ms: u64,
}

impl Default for Calendrier {
    fn default() -> Self {
        Self::nouveau()
    }
}

impl Calendrier {
    /// Un calendrier neuf est DU tout de suite.
    ///
    /// C'est ce qui permet a un support branche tard d'etre servi tot : la
    /// question « peut-on poser ? » se pose des la premiere seconde, et la
    /// reponse « pas encore » ne coute plus une periode entiere.
    pub const fn nouveau() -> Self {
        Self {
            prochain_ms: 0,
            reports_consecutifs: 0,
            echecs_consecutifs: 0,
            poses: 0,
            dernier_ok_ms: 0,
            dernier_essai_ms: 0,
        }
    }

    pub fn echeance_ms(&self) -> u64 {
        self.prochain_ms
    }

    pub fn poses(&self) -> u64 {
        self.poses
    }

    pub fn dernier_ok_ms(&self) -> u64 {
        self.dernier_ok_ms
    }

    pub fn reports_consecutifs(&self) -> u32 {
        self.reports_consecutifs
    }

    /// Consulte le calendrier. NE POSE RIEN et ne consomme aucune echeance.
    ///
    /// L'appelant qui recoit `Pose` doit rendre compte par `pose_reussie` ou
    /// `pose_echouee` : c'est ce compte rendu, et lui seul, qui fait avancer
    /// l'echeance. Ne pas rendre compte laisse l'echeance atteinte, donc la
    /// tentative se represente -- ce qui est le bon defaut, puisqu'un
    /// checkpoint qui n'a pas eu lieu doit avoir lieu.
    pub fn consulte(&mut self, maintenant_ms: u64, support_pret: bool) -> Decision {
        if maintenant_ms < self.prochain_ms {
            return Decision::Attendre {
                restant_ms: self.prochain_ms - maintenant_ms,
            };
        }
        if !support_pret {
            // L'ECHEANCE N'EST PAS CONSOMMEE. Elle est repoussee de peu, et
            // c'est toute la correction.
            let report = if self.reports_consecutifs == 0 {
                REPORT_COURT_MS
            } else {
                REPORT_PALIER_MS
            };
            self.reports_consecutifs = self.reports_consecutifs.saturating_add(1);
            self.prochain_ms = maintenant_ms.saturating_add(report);
            return Decision::Reporte {
                raison: Raison::SansSupport,
                prochain_ms: self.prochain_ms,
            };
        }
        self.dernier_essai_ms = maintenant_ms;
        Decision::Pose
    }

    /// Le checkpoint a ete pose et confirme.
    pub fn pose_reussie(&mut self, maintenant_ms: u64) {
        self.poses = self.poses.saturating_add(1);
        self.dernier_ok_ms = maintenant_ms;
        self.reports_consecutifs = 0;
        self.echecs_consecutifs = 0;
        self.prochain_ms = maintenant_ms.saturating_add(PERIODE_MS);
    }

    /// Le support repondait, et le checkpoint a echoue.
    pub fn pose_echouee(&mut self, maintenant_ms: u64) -> Decision {
        self.echecs_consecutifs = self.echecs_consecutifs.saturating_add(1);
        let report = if self.echecs_consecutifs <= ECHECS_AVANT_CADENCE_NORMALE {
            REPORT_PALIER_MS
        } else {
            // Marteler une cle defaillante ne la repare pas.
            PERIODE_MS
        };
        self.prochain_ms = maintenant_ms.saturating_add(report);
        Decision::Reporte {
            raison: Raison::EchecEcriture,
            prochain_ms: self.prochain_ms,
        }
    }

    /// Rend l'echeance immediatement due. Pour un checkpoint demande a la main.
    pub fn force(&mut self) {
        self.prochain_ms = 0;
    }
}
