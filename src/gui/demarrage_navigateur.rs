//! Quand lancer le navigateur, une fois le bureau affiche.
//!
//! # Le defaut, releve le 16 septembre 2026
//!
//! Le lancement se declenchait cinq cents millisecondes apres la premiere
//! trame du bureau, sur le seul critere du temps. Le releve physique montre
//! ce que cela donne :
//!
//!     t=5799 ms  bureau-premiere-trame
//!     t~6500 ms  le lien Ethernet monte enfin (autonegociation cuivre)
//!     t=6799 ms  navigateur-demande  -- resolveur=NON-CONFIGURE
//!
//! Le navigateur part trois cents millisecondes apres que le lien soit monte,
//! donc avant que le moindre bail DHCP ait pu revenir. Il lit son resolveur
//! une seule fois, a l'exec, et le garde pour la vie : la session entiere se
//! passe ensuite sans DNS.
//!
//! # La regle
//!
//! Attendre le resolveur, mais SEULEMENT quand il peut arriver. Lien bas, pas
//! d'attente : une machine hors reseau doit ouvrir son navigateur tout de
//! suite, et une page locale n'a besoin de personne. Lien haut, on laisse au
//! bail le temps de revenir, borne -- un reseau sans serveur DHCP ne doit pas
//! retenir le bureau indefiniment.
//!
//! # Pourquoi ce module est PUR
//!
//! Aucun `use crate::`, aucun `unsafe`, le temps en parametre. Les quatre
//! situations se verifient sur machine hote ; sur la machine elles ne se
//! distinguent qu'a une seconde pres, une fois par demarrage.

/// Ce qu'il faut faire a cet instant.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Decision {
    /// Le bureau vient de s'afficher : le laisser se poser.
    LaisserLeBureauSePoser = 0,
    /// Le lien peut encore fournir un resolveur : attendre.
    AttendreLeResolveur = 1,
    /// Resolveur configure : c'est le bon moment.
    LancerReseauPret = 2,
    /// Aucune carte : attendre ne rapporterait rien.
    LancerSansReseau = 3,
    /// Le lien est la mais le bail ne vient pas. On n'attend pas plus.
    LancerDelaiEcoule = 4,
}

impl Decision {
    /// Faut-il lancer maintenant ?
    pub fn lance(self) -> bool {
        matches!(
            self,
            Decision::LancerReseauPret
                | Decision::LancerSansReseau
                | Decision::LancerDelaiEcoule
        )
    }

    pub fn nom(self) -> &'static str {
        match self {
            Decision::LaisserLeBureauSePoser => "bureau-se-pose",
            Decision::AttendreLeResolveur => "attente-resolveur",
            Decision::LancerReseauPret => "reseau-pret",
            Decision::LancerSansReseau => "sans-reseau",
            Decision::LancerDelaiEcoule => "delai-ecoule",
        }
    }
}

/// Temps de repos du bureau avant toute tentative, en millisecondes.
pub const REPOS_BUREAU_MS: u64 = 500;
/// Au-dela, on lance meme sans resolveur. Un reseau sans serveur DHCP ne doit
/// pas retenir le bureau : le navigateur s'ouvrira sur sa page locale.
pub const ATTENTE_MAXIMALE_MS: u64 = 8_000;

pub fn decide(
    depuis_premiere_trame_ms: u64,
    // BOUCHAUD_LIEN_BAS_N_EST_PAS_SANS_CABLE_V1
    //
    // Ce parametre etait `lien: bool`, lu depuis `net::connecte()`. Le releve
    // du 16 septembre 18:31 montre ce que cela donne :
    //
    //     BOUCHAUD_NAVIGATEUR_DEPART decision=sans-reseau t_ms=1322 lien=0
    //
    // « Sans reseau » sur une machine dont le cable etait branche et dont le
    // lien est monte quelques secondes plus tard a 1 Gbit/s. L'autonegociation
    // cuivre prend environ trois secondes : pendant ce temps `connecte()` est
    // faux, et le lire comme « pas de cable » supprimait exactement l'attente
    // qu'on venait d'ajouter.
    //
    // Ce qu'il faut savoir n'est pas « le lien est-il monte MAINTENANT », mais
    // « un bail peut-il encore arriver ». Seule l'absence de carte -- ou une
    // carte refusee -- repond non.
    bail_possible: bool,
    resolveur_pret: bool,
    repos_ms: u64,
    attente_maximale_ms: u64,
) -> Decision {
    if depuis_premiere_trame_ms < repos_ms {
        return Decision::LaisserLeBureauSePoser;
    }
    if resolveur_pret {
        return Decision::LancerReseauPret;
    }
    // L'ORDRE COMPTE. Tester le delai avant la carte ferait attendre huit
    // secondes une machine sans carte, pour un bail qui ne peut pas arriver.
    if !bail_possible {
        return Decision::LancerSansReseau;
    }
    if depuis_premiere_trame_ms >= attente_maximale_ms {
        return Decision::LancerDelaiEcoule;
    }
    Decision::AttendreLeResolveur
}

// BOUCHAUD_NAVIGATEUR_DEMARRAGE_EXPLICITE_V1
//
// # Depuis p18, le navigateur ne part plus tout seul
//
// Le bureau le demarre sur DEMANDE -- bouton Services, BRDP `browser start`
// -- pour que le diagnostic reseau soit disponible avant sa charge processeur
// et memoire. Mais une demande peut arriver AVANT le bail : un clic des la
// premiere trame, un script BRDP qui enchaine apres un redemarrage. Le defaut
// du 16 septembre n'a besoin de rien d'autre : le navigateur lit son
// resolveur UNE FOIS, a l'exec, et une session partie trop tot se passe
// entiere sans DNS.
//
// `decide` repond toujours a la meme question -- le reseau a-t-il eu sa
// chance -- mais ce n'est plus lui qui LANCE. Le portail retient une demande
// de demarrage tant que `decide` n'a pas dit « lancer », et la relache au
// premier tour ou il le dit. Sur un reseau deja pret, rien ne change : la
// demande part au tour meme ou elle est lue.

/// Une commande de service, telle que le portail la voit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Commande {
    Aucune,
    Demarrer,
    Arreter,
    /// Toute autre commande -- le redemarrage en deux phases. Elle traverse :
    /// sa phase d'arret n'a rien a attendre, et sa phase de demarrage revient
    /// au tour suivant comme un `Demarrer`, qui passe alors par le portail.
    Autre,
}

/// Au plus UNE demande de demarrage retenue, et pas de file : la derniere
/// commande l'emporte, comme dans `gui::services`, dont la boite aux lettres
/// ne garde elle aussi que la derniere.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Portail {
    retenue: bool,
}

impl Portail {
    pub const fn neuf() -> Self {
        Portail { retenue: false }
    }

    /// Une demande de demarrage attend-elle que le reseau ait eu sa chance ?
    pub fn retient(&self) -> bool {
        self.retenue
    }

    /// La commande a executer CE tour.
    ///
    /// `ouvert` : `decide` a deja dit « lancer » depuis la premiere trame du
    /// bureau. Une fois ouvert, le portail ne se referme pas -- l'arbitrage
    /// porte sur le PREMIER bail, pas sur chaque demarrage.
    pub fn filtre(&mut self, commande: Commande, ouvert: bool) -> Commande {
        match commande {
            Commande::Demarrer if !ouvert => {
                self.retenue = true;
                Commande::Aucune
            }
            Commande::Demarrer => {
                self.retenue = false;
                Commande::Demarrer
            }
            // Un arret n'attend JAMAIS, et il annule une demande retenue :
            // c'est la derniere volonte exprimee. Relancer apres coup ce
            // qu'on vient de demander d'arreter serait pire que l'attente.
            Commande::Arreter => {
                self.retenue = false;
                Commande::Arreter
            }
            Commande::Autre => Commande::Autre,
            Commande::Aucune if self.retenue && ouvert => {
                self.retenue = false;
                Commande::Demarrer
            }
            Commande::Aucune => Commande::Aucune,
        }
    }
}
