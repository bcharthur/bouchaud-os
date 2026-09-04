//! Jauge de chargement : au bout de combien de temps une page s'affiche-t-elle.
//!
//! # Pourquoi cette mesure existe
//!
//! « Le navigateur est lent a demarrer » et « la page met longtemps » sont des
//! phrases, pas des mesures. Le journal du noyau contient des fautes de page,
//! des trames utiles et des latences d'ordonnancement -- rien qui reponde a la
//! seule question que se pose quelqu'un devant l'ecran : COMBIEN DE TEMPS.
//!
//! Ce module produit ce nombre, et le compositeur le dessine.
//!
//! # Ce que la jauge mesure exactement
//!
//! Le noyau ne voit pas le DOM. Il ne peut donc pas rendre `DOMContentLoaded`,
//! et pretendre le contraire serait un mensonge affiche a l'ecran. Ce qu'il
//! voit, et qui est precisement ce que l'oeil voit :
//!
//!   * **demarrage** : du lancement du processus a sa PREMIERE trame ;
//!   * **chargement** : du debut d'une rafale de trames a son repos.
//!
//! La duree affichee est donc « le temps pendant lequel l'image a change ».
//! C'est la duree perceptible, celle que l'utilisateur chronometre lui-meme en
//! regardant l'ecran -- pas une metrique du moteur Web.
//!
//! # Ce qui distingue un chargement d'un defilement
//!
//! Les deux produisent une rafale de trames. Un seul critere les separe, et le
//! compositeur le possede deja : l'ENTREE. Une rafale qui commence dans les
//! [`FENETRE_INTERACTION_MS`] qui suivent une touche, un clic ou un cran de
//! molette est une interaction ; elle ne demarre aucun chronometre. Sans ce
//! test, la jauge se serait rallumee a chaque coup de molette.
//!
//! # Aucune horloge, aucun etat global
//!
//! Chaque methode recoit la date. Le module est donc une machine a etats pure,
//! exercable sur l'hote sans QEMU -- voir `tools/gui/test-jauge.sh`.
//!
//! BOUCHAUD_C13_JAUGE_DE_CHARGEMENT_V1

/// Silence de trames au-dela duquel un chargement est considere acheve.
///
/// Une page qui se peint progressivement laisse des trous : trop court, la
/// jauge coupe le chronometre au milieu du rendu et annonce une duree fausse
/// par defaut -- le sens ou une mesure de performance ment le plus volontiers.
pub const SEUIL_REPOS_MS: u64 = 500;

/// Une rafale qui commence dans cette fenetre apres une entree est une
/// interaction, pas un chargement.
pub const FENETRE_INTERACTION_MS: u64 = 250;

/// Duree pendant laquelle le resultat reste affiche apres le chargement.
pub const AFFICHAGE_MS: u64 = 4_000;

/// Au-dela, ce n'est plus un chargement : c'est une page qui s'anime.
///
/// Une video, un carrousel ou une animation CSS produisent des trames sans
/// jamais se taire. Le critere de repos ne peut alors pas conclure, et le
/// chronometre tournerait indefiniment. Passe ce plafond la jauge se TAIT
/// plutot que d'afficher un nombre qui ne mesure plus rien : elle redeviendra
/// bavarde des que la page se sera reellement reposee. Aucune duree fausse
/// n'est donc jamais affichee -- seulement, parfois, aucune duree.
///
/// Le demarrage n'est pas concerne : un navigateur qui met quarante secondes a
/// peindre sa premiere trame est exactement ce qu'il faut montrer.
pub const PLAFOND_CHARGE_MS: u64 = 30_000;

/// Demi-vie de la progression pendant le demarrage du navigateur.
pub const DEMI_VIE_DEMARRAGE_MS: u64 = 4_000;

/// Demi-vie de la progression pendant le chargement d'une page.
pub const DEMI_VIE_PAGE_MS: u64 = 1_200;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    /// Le processus est lance ; aucune trame n'est encore arrivee.
    Demarrage,
    /// Rien ne charge : la jauge ne se dessine pas.
    Repos,
    /// L'image change : le chronometre tourne.
    Charge,
    /// Le chargement est acheve ; sa duree reste lisible [`AFFICHAGE_MS`].
    Termine,
}

#[derive(Clone, Copy, Debug)]
pub struct Jauge {
    phase: Phase,
    debut_ms: u64,
    derniere_trame_ms: u64,
    derniere_entree_ms: u64,
    fin_ms: u64,
    duree_ms: u64,
    demarrage_ms: u64,
}

impl Jauge {
    /// Une jauge neuve, au moment ou le processus est lance.
    pub const fn neuve(maintenant_ms: u64) -> Self {
        Self {
            phase: Phase::Demarrage,
            debut_ms: maintenant_ms,
            derniere_trame_ms: 0,
            derniere_entree_ms: 0,
            fin_ms: 0,
            duree_ms: 0,
            demarrage_ms: 0,
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Duree du demarrage, une fois la premiere trame recue.
    ///
    /// `None` tant que le navigateur n'a rien peint. Le champ est arrondi a
    /// une milliseconde minimum a l'enregistrement, ce qui rend zero
    /// impossible : « pas encore mesure » et « mesure a zero » ne peuvent donc
    /// pas se confondre, y compris apres un demarrage abandonne.
    pub fn demarrage_ms(&self) -> Option<u64> {
        if self.demarrage_ms == 0 { None } else { Some(self.demarrage_ms) }
    }

    /// Une entree utilisateur vient d'etre transmise au client.
    pub fn note_entree(&mut self, maintenant_ms: u64) {
        self.derniere_entree_ms = maintenant_ms;
    }

    /// Le client a change le titre de sa fenetre.
    ///
    /// C'est le seul signal de NAVIGATION que le protocole porte. Il est plus
    /// fort que la detection de rafale : il demarre un chronometre meme si
    /// l'utilisateur vient de taper -- il vient justement de valider une URL.
    pub fn note_titre(&mut self, maintenant_ms: u64) {
        if self.phase == Phase::Demarrage {
            return;
        }
        self.commence(maintenant_ms);
    }

    /// Le client a presente une trame.
    pub fn note_trame(&mut self, maintenant_ms: u64) {
        match self.phase {
            Phase::Demarrage => {
                self.duree_ms = maintenant_ms.saturating_sub(self.debut_ms);
                self.demarrage_ms = self.duree_ms.max(1);
                self.fin_ms = maintenant_ms;
                self.phase = Phase::Termine;
            }
            // Une rafale est une suite maximale de trames separees de moins
            // de [`SEUIL_REPOS_MS`]. Les deux phases au repos appliquent donc
            // la MEME regle : une trame ne commence un chargement que si elle
            // ouvre une rafale, et si aucune entree ne l'explique.
            //
            // Sans le test de rafale, une page qui s'anime sans fin rallumait
            // la jauge toutes les [`AFFICHAGE_MS`], en plein milieu d'un flot
            // de trames ininterrompu.
            Phase::Repos | Phase::Termine => {
                if self.hors_interaction(maintenant_ms) && self.ouvre_une_rafale(maintenant_ms) {
                    self.commence(maintenant_ms);
                }
            }
            Phase::Charge => {}
        }
        self.derniere_trame_ms = maintenant_ms;
    }

    /// Le temps passe. Seule methode qui puisse ACHEVER un chargement : la fin
    /// d'une rafale est un non-evenement, personne ne l'annonce.
    pub fn tic(&mut self, maintenant_ms: u64) {
        match self.phase {
            Phase::Charge => {
                if maintenant_ms.saturating_sub(self.derniere_trame_ms) >= SEUIL_REPOS_MS {
                    // La duree s'arrete a la DERNIERE TRAME, jamais a l'instant
                    // ou le repos est constate : sinon chaque page se verrait
                    // facturer le seuil, et la mesure dependrait de la cadence
                    // a laquelle le compositeur pense a appeler `tic`.
                    self.duree_ms = self.derniere_trame_ms.saturating_sub(self.debut_ms);
                    self.fin_ms = self.derniere_trame_ms;
                    self.phase = Phase::Termine;
                } else if maintenant_ms.saturating_sub(self.debut_ms) >= PLAFOND_CHARGE_MS {
                    self.phase = Phase::Repos;
                }
            }
            Phase::Termine => {
                if maintenant_ms.saturating_sub(self.fin_ms) >= AFFICHAGE_MS {
                    self.phase = Phase::Repos;
                }
            }
            Phase::Demarrage | Phase::Repos => {}
        }
    }

    /// Le compositeur a renonce a attendre les trames de ce client.
    ///
    /// Sans cela, un client declare muet laisserait un chronometre de
    /// demarrage tourner indefiniment : la jauge afficherait une seconde de
    /// plus toutes les secondes, pour toujours.
    pub fn abandonne_demarrage(&mut self) {
        if self.phase == Phase::Demarrage {
            self.phase = Phase::Repos;
        }
    }

    /// Faut-il dessiner quelque chose ?
    pub fn visible(&self) -> bool {
        self.phase != Phase::Repos
    }

    /// Le chronometre tourne-t-il ? (le compositeur doit alors se reveiller)
    pub fn en_cours(&self) -> bool {
        matches!(self.phase, Phase::Demarrage | Phase::Charge)
    }

    /// La duree a afficher, en millisecondes.
    pub fn duree_affichee_ms(&self, maintenant_ms: u64) -> u64 {
        match self.phase {
            Phase::Demarrage | Phase::Charge => maintenant_ms.saturating_sub(self.debut_ms),
            Phase::Termine => self.duree_ms,
            Phase::Repos => 0,
        }
    }

    /// Remplissage de la barre, de 0 a 100.
    ///
    /// # Pourquoi une asymptote et pas une fraction
    ///
    /// Personne ne connait le total : ni le noyau, ni le moteur Web, ni le
    /// serveur d'en face. Une barre « a 70 % » serait donc inventee. Celle-ci
    /// avance vite au debut, ralentit, et n'atteint JAMAIS 100 avant que le
    /// chargement soit reellement acheve. Elle ne promet rien qu'elle ne
    /// puisse tenir, et elle continue de bouger tant que ca charge -- ce qui
    /// est exactement l'information demandee.
    pub fn progression(&self, maintenant_ms: u64) -> u8 {
        match self.phase {
            Phase::Repos => 0,
            Phase::Termine => 100,
            Phase::Demarrage => asymptote(
                maintenant_ms.saturating_sub(self.debut_ms),
                DEMI_VIE_DEMARRAGE_MS,
            ),
            Phase::Charge => asymptote(
                maintenant_ms.saturating_sub(self.debut_ms),
                DEMI_VIE_PAGE_MS,
            ),
        }
    }

    /// Cette trame ouvre-t-elle une rafale, ou prolonge-t-elle la precedente ?
    fn ouvre_une_rafale(&self, maintenant_ms: u64) -> bool {
        self.derniere_trame_ms == 0
            || maintenant_ms.saturating_sub(self.derniere_trame_ms) >= SEUIL_REPOS_MS
    }

    fn hors_interaction(&self, maintenant_ms: u64) -> bool {
        self.derniere_entree_ms == 0
            || maintenant_ms.saturating_sub(self.derniere_entree_ms) >= FENETRE_INTERACTION_MS
    }

    fn commence(&mut self, maintenant_ms: u64) {
        self.phase = Phase::Charge;
        self.debut_ms = maintenant_ms;
        self.derniere_trame_ms = maintenant_ms;
    }
}

/// `100 * e / (e + demi_vie)`, bornee a 99 tant que ce n'est pas fini.
///
/// Entierement entiere : le FPU appartient a la tache interrompue, et le
/// compositeur tourne dans le noyau.
fn asymptote(ecoule_ms: u64, demi_vie_ms: u64) -> u8 {
    let denominateur = ecoule_ms.saturating_add(demi_vie_ms.max(1));
    let brut = ecoule_ms.saturating_mul(100) / denominateur.max(1);
    brut.min(99) as u8
}

// ─── Mise en forme ─────────────────────────────────────────────────────────

/// Une duree rendue lisible, sans allocation.
///
/// Le rendu d'une trame ne doit pas dependre de l'allocateur : la jauge se
/// dessine precisement quand le systeme est charge.
pub struct Duree {
    octets: [u8; 16],
    longueur: usize,
}

impl Duree {
    pub fn as_str(&self) -> &str {
        // `ecris` n'ecrit que de l'ASCII.
        core::str::from_utf8(&self.octets[..self.longueur]).unwrap_or("")
    }
}

impl core::fmt::Write for Duree {
    fn write_str(&mut self, texte: &str) -> core::fmt::Result {
        for octet in texte.as_bytes() {
            if self.longueur >= self.octets.len() {
                return Err(core::fmt::Error);
            }
            self.octets[self.longueur] = *octet;
            self.longueur += 1;
        }
        Ok(())
    }
}

/// « 612 ms », « 1,84 s », « 12,3 s », « 1 min 04 s ».
///
/// La virgule est celle de la langue de l'interface, pas un point.
pub fn formate_duree(ms: u64) -> Duree {
    use core::fmt::Write;
    let mut sortie = Duree { octets: [0; 16], longueur: 0 };
    let _ = if ms < 1_000 {
        write!(sortie, "{} ms", ms)
    } else if ms < 10_000 {
        write!(sortie, "{},{:02} s", ms / 1_000, (ms % 1_000) / 10)
    } else if ms < 60_000 {
        write!(sortie, "{},{} s", ms / 1_000, (ms % 1_000) / 100)
    } else {
        write!(sortie, "{} min {:02} s", ms / 60_000, (ms % 60_000) / 1_000)
    };
    sortie
}
