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
