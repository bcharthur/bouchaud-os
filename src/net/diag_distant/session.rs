//! L'etat d'une connexion BRDP : ce qui attend d'etre traite, et ce qui reste
//! a envoyer.
//!
//! # Les deux defauts que ce module corrige
//!
//! **Une seule commande par segment.** Le decoupeur sait rendre plusieurs
//! lignes ; le serveur n'en gardait qu'une :
//!
//! ```rust,ignore
//! if a_traiter.is_none() { a_traiter = Some(copie); }   // les suivantes : perdues
//! ```
//!
//! Un client qui envoie `status`, `audit status` et `blackbox status` dans le
//! meme `send` perdait les deux dernieres -- silencieusement, et sans que rien
//! ne le lui dise. TCP est un flux : rien n'interdit au noyau distant de
//! coller trois commandes dans un segment, et c'est meme ce qu'il fait des que
//! le client enchaine.
//!
//! **`events tail N` n'envoyait rien.** Le mode etait un booleen : `Watch`
//! vrai ou faux. `Tail` posait le curseur puis mettait le booleen a FAUX,
//! donc aucune routine ne vidait jamais les evenements demandes. La commande
//! repondait `{"ok":true}` et s'arretait la.
//!
//! # Pur
//!
//! Pas de chaussette, pas d'anneau, pas d'horloge. Des octets et des compteurs.
//! La file pleine, l'ordre des commandes et l'epuisement d'un `tail` se
//! contredisent donc en test hote, sans pile reseau.

use super::brdp::{Decoupeur, Morceau, LIGNE_MAX};

/// La sentinelle qui fait passer « ligne trop longue » par la file.
///
/// # Pourquoi une sentinelle plutot qu'un drapeau
///
/// La reponse a une ligne trop longue doit arriver A SA PLACE dans la suite
/// des reponses, sinon le client la rattache a la mauvaise commande. Un
/// drapeau a cote de la file la ferait sortir en avance ou en retard ; en la
/// faisant passer PAR la file, l'ordre des reponses suit l'ordre des lignes
/// recues, qui est le seul ordre que le client puisse verifier.
///
/// Ces octets ne peuvent pas etre une vraie commande : le protocole n'accepte
/// que des objets JSON, donc une ligne utile commence par `{`.
pub const LIGNE_TROP_LONGUE: &[u8] = b"\x00trop-longue";

/// Commandes retenues en attendant leur tour.
///
/// # Pourquoi huit, et pourquoi une borne
///
/// Le serveur SERIALISE : il repond a une commande avant de lire la suivante,
/// parce qu'il n'a qu'un tampon de reponse. Ce qui arrive pendant ce temps
/// doit donc etre retenu quelque part, et ce quelque part doit etre borne --
/// un `Vec` qui grandit avec ce qu'un pair envoie est une panne memoire
/// declenchable a distance, sur la machine meme qu'on essaie d'observer.
///
/// Huit commandes representent quatre kibioctets. Aucun client raisonnable
/// n'en envoie autant sans lire ses reponses ; celui qui le fait est refuse,
/// et il l'APPREND.
pub const COMMANDES_EN_ATTENTE: usize = 8;

/// Une file bornee de lignes de commande, dans l'ordre d'arrivee.
pub struct FileCommandes {
    lignes: [[u8; LIGNE_MAX]; COMMANDES_EN_ATTENTE],
    longueurs: [usize; COMMANDES_EN_ATTENTE],
    tete: usize,
    occupation: usize,
    /// Commandes refusees faute de place. Le client en est informe.
    refusees: u64,
}

impl Default for FileCommandes {
    fn default() -> Self {
        Self::neuve()
    }
}

impl FileCommandes {
    pub const fn neuve() -> Self {
        Self {
            lignes: [[0; LIGNE_MAX]; COMMANDES_EN_ATTENTE],
            longueurs: [0; COMMANDES_EN_ATTENTE],
            tete: 0,
            occupation: 0,
            refusees: 0,
        }
    }

    pub fn occupation(&self) -> usize {
        self.occupation
    }

    pub fn est_vide(&self) -> bool {
        self.occupation == 0
    }

    pub fn pleine(&self) -> bool {
        self.occupation == COMMANDES_EN_ATTENTE
    }

    pub fn refusees(&self) -> u64 {
        self.refusees
    }

    /// Range une ligne. Rend `false` si la file est pleine.
    ///
    /// LE PLUS ANCIEN N'EST PAS ECRASE. Un anneau d'evenements peut se
    /// permettre d'ecraser -- le lecteur l'apprend et se recale. Une file de
    /// COMMANDES, non : ecraser la plus ancienne ferait repondre au client
    /// dans un ordre qu'il n'a pas demande, ou pas du tout, sans qu'il puisse
    /// le distinguer d'une perte reseau. On refuse la plus RECENTE, et on le
    /// dit.
    pub fn pousse(&mut self, ligne: &[u8]) -> bool {
        if ligne.len() > LIGNE_MAX {
            return false;
        }
        if self.pleine() {
            self.refusees = self.refusees.saturating_add(1);
            return false;
        }
        let place = (self.tete + self.occupation) % COMMANDES_EN_ATTENTE;
        self.lignes[place][..ligne.len()].copy_from_slice(ligne);
        self.longueurs[place] = ligne.len();
        self.occupation += 1;
        true
    }

    /// Retire la plus ancienne ligne dans `sortie`. Rend sa longueur.
    pub fn retire(&mut self, sortie: &mut [u8; LIGNE_MAX]) -> Option<usize> {
        if self.occupation == 0 {
            return None;
        }
        let n = self.longueurs[self.tete];
        sortie[..n].copy_from_slice(&self.lignes[self.tete][..n]);
        self.tete = (self.tete + 1) % COMMANDES_EN_ATTENTE;
        self.occupation -= 1;
        Some(n)
    }

    /// Oublie tout. Pour une nouvelle connexion.
    ///
    /// `refusees` SURVIT : c'est un compteur de service, pas un etat de
    /// connexion, et le remettre a zero effacerait la trace d'un client qui
    /// maltraite le serveur a chaque reconnexion.
    pub fn purge(&mut self) {
        self.tete = 0;
        self.occupation = 0;
    }
}

/// Ce que la connexion attend du flux d'evenements.
///
/// # Pourquoi ce n'est plus un booleen
///
/// Un booleen ne sait dire que « suivre » ou « ne pas suivre ». `events tail N`
/// est un troisieme etat : envoyer exactement N evenements, puis s'arreter. Le
/// coder par « ne pas suivre » revenait a ne rien envoyer du tout, et c'est
/// precisement ce que faisait la premiere redaction.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ModeEvenements {
    /// Rien a envoyer.
    #[default]
    Aucun,
    /// Envoyer au plus `restant` evenements, puis revenir a `Aucun`.
    Queue { restant: u32 },
    /// Suivre le flux indefiniment.
    Suivi,
}

impl ModeEvenements {
    /// Ce mode attend-il encore quelque chose ?
    pub fn actif(&self) -> bool {
        !matches!(self, ModeEvenements::Aucun)
    }

    /// Combien d'evenements ce mode autorise encore a envoyer, au plus.
    ///
    /// `Suivi` n'a pas de borne propre ; l'appelant lui applique la sienne,
    /// par tour de boucle.
    pub fn budget(&self, borne_par_tour: u32) -> u32 {
        match self {
            ModeEvenements::Aucun => 0,
            ModeEvenements::Queue { restant } => (*restant).min(borne_par_tour),
            ModeEvenements::Suivi => borne_par_tour,
        }
    }

    /// Compte `combien` evenements envoyes, et rend le mode qui suit.
    ///
    /// Un `Queue` epuise redevient `Aucun` : sans cela, `events tail 5`
    /// continuerait a servir le flux comme un `watch`, et la commande ne
    /// voudrait plus rien dire.
    pub fn consomme(self, combien: u32) -> Self {
        match self {
            ModeEvenements::Aucun => ModeEvenements::Aucun,
            ModeEvenements::Suivi => ModeEvenements::Suivi,
            ModeEvenements::Queue { restant } => {
                let reste = restant.saturating_sub(combien);
                if reste == 0 {
                    ModeEvenements::Aucun
                } else {
                    ModeEvenements::Queue { restant: reste }
                }
            }
        }
    }

    /// Le mode s'arrete-t-il quand l'anneau n'a plus rien a donner ?
    ///
    /// `Queue` oui : `events tail 100` sur un anneau qui n'en contient que
    /// vingt rend vingt evenements et se tait, au lieu d'attendre indefiniment
    /// les quatre-vingts qui n'existent pas. `Suivi` non, c'est son travail.
    pub fn finit_sur_vide(&self) -> bool {
        matches!(self, ModeEvenements::Queue { .. })
    }
}

/// Verse un segment de flux dans la file, DANS L'ORDRE D'ARRIVEE.
///
/// Rend `true` si au moins une ligne a ete refusee faute de place.
///
/// # Pourquoi la ligne trop longue est poussee ICI et pas apres
///
/// Une premiere redaction levait un drapeau dans le rappel et poussait la
/// sentinelle APRES le decoupage. Un seul segment contenant
///
/// ```text
/// <ligne de six cents octets>\n
/// {"cmd":"status"}\n
/// ```
///
/// mettait donc `status` en file d'abord, puis la sentinelle : le client
/// recevait la reponse a `status`, PUIS `ligne-trop-longue`. Il rattache les
/// reponses aux commandes dans l'ordre d'emission -- c'est le seul ordre dont
/// il dispose -- donc il attribuait chaque reponse a la mauvaise commande,
/// pour tout le reste de la connexion.
///
/// La sentinelle prend sa place DANS le flux, a l'instant ou le decoupeur rend
/// le verdict. L'ordre des reponses suit alors l'ordre des lignes recues, ce
/// qui est la seule promesse que le protocole ait a tenir.
///
/// # Pur
///
/// Deux emprunts disjoints et des octets. Le serveur passe ses propres champs ;
/// une epreuve hote passe un decoupeur et une file a elle, et l'ordre se
/// contredit sans pile reseau.
pub fn avale(decoupeur: &mut Decoupeur, file: &mut FileCommandes, octets: &[u8]) -> bool {
    let mut deborde = false;
    decoupeur.pousse(octets, |m| {
        let ligne: &[u8] = match m {
            Morceau::Ligne(l) => l,
            Morceau::TropLongue => LIGNE_TROP_LONGUE,
        };
        if !file.pousse(ligne) {
            deborde = true;
        }
    });
    deborde
}
