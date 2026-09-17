//! La file de trames d'un consommateur, PURE : qui recoit quoi, et ce qui se
//! perd quand personne ne lit.
//!
//! # Le defaut que ce module ferme
//!
//! Deux piles lisaient l'anneau de la carte : le routage maison
//! (`draine_verrouille`) et le peripherique `smoltcp` (`E1000Device::receive`).
//! Le commentaire de tete de `smol_device.rs` le disait deja -- « les deux ne
//! doivent JAMAIS tourner en meme temps » -- et rien ne l'empechait.
//!
//! Ce n'est pas seulement une course memoire. C'est une question de PROPRIETE
//! DES PAQUETS : une reponse ARP, une reponse DNS ou un segment TCP retire de
//! la carte par le mauvais consommateur n'est jamais vu par l'autre pile, et
//! une reponse ARP ne se retransmet pas.
//!
//! Le releve TRIGKEY du 17 septembre en porte la signature exacte : pendant
//! trente secondes, avec Ladybird vivant,
//!
//! ```text
//! [NET-ROUTAGE] trames=100 arp=13 dhcp=2 arp_resolus=1 arp_echoues=0
//! [NET-TCP]     poignees=0 echantillons_rtt=0 syn_retransmis=0
//! ```
//!
//! ne bouge plus d'un seul compteur -- alors que le lien porte a un gigabit.
//!
//! # L'invariant
//!
//! UNE seule fonction lit physiquement l'anneau. Elle repartit ensuite chaque
//! trame vers ceux a qui elle appartient :
//!
//! ```text
//!   NIC RX
//!     |
//!     v
//!   ingress unique
//!     |
//!     +--> ARP maison
//!     +--> DHCP
//!     +--> files IPv4 maison
//!     +--> file smoltcp   <-- ce module
//! ```
//!
//! # Pourquoi une COPIE, et non un aiguillage
//!
//! On pourrait croire qu'il suffit d'envoyer a `smoltcp` ce qui lui est
//! destine. C'est faux : `smoltcp` tient sa propre table de voisins, et sans
//! les reponses ARP il ne sait a qui remettre ses segments. Il lui faut donc
//! le meme flux que la pile maison, et non un sous-ensemble.
//!
//! La file est BORNEE et ce qu'elle perd se compte. Une file sans borne dans
//! un noyau, c'est une panne memoire deguisee en fonctionnalite.

/// Trames retenues par consommateur.
///
/// Trente-deux : au-dela d'une rafale d'un aller-retour TCP, et tres au-dessous
/// de ce qu'une file non bornee couterait. `smoltcp` est scrute en boucle
/// serree pendant une requete ; ce qui s'accumule ici ne represente que le
/// temps d'un tour de boucle.
pub const TRAMES: usize = 32;

/// Taille d'une trame retenue : MTU Ethernet plus la marge d'en-tete.
pub const TAILLE: usize = 1600;

/// Ce que la file a vu passer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Compteurs {
    /// Trames deposees et retirees.
    pub posees: u64,
    pub retirees: u64,
    /// Trames refusees faute de place. C'est la SEULE perte possible, et elle
    /// se voit.
    pub perdues_pleine: u64,
    /// Trames refusees parce que trop longues pour un tampon.
    pub perdues_taille: u64,
    /// Occupation maximale atteinte. Dit si la borne est la bonne.
    pub occupation_max: usize,
}

/// Une file de trames bornee, sans allocation.
///
/// Elle n'est pas thread-safe par elle-meme : l'appelant la protege avec le
/// verrou qui protege deja l'anneau de la carte. C'est voulu -- le point de
/// ce module est qu'il n'y ait QU'UN endroit ou l'on synchronise.
pub struct FileTrames {
    tampons: [[u8; TAILLE]; TRAMES],
    longueurs: [usize; TRAMES],
    tete: usize,
    occupation: usize,
    compteurs: Compteurs,
    /// Personne ne lit cette file : inutile d'y recopier quoi que ce soit.
    abonnee: bool,
}

impl FileTrames {
    pub const fn neuve() -> Self {
        Self {
            tampons: [[0u8; TAILLE]; TRAMES],
            longueurs: [0; TRAMES],
            tete: 0,
            occupation: 0,
            compteurs: Compteurs {
                posees: 0,
                retirees: 0,
                perdues_pleine: 0,
                perdues_taille: 0,
                occupation_max: 0,
            },
            abonnee: false,
        }
    }

    /// Ouvre la file. Tant qu'elle est fermee, `pose` ne recopie rien.
    ///
    /// L'abonnement EXISTE pour que le cas courant -- aucune pile `smoltcp` en
    /// cours -- ne paie pas une copie de trame par paquet recu.
    pub fn abonne(&mut self) {
        self.abonnee = true;
        self.vide();
    }

    /// Ferme la file et jette ce qu'elle retenait.
    ///
    /// Les trames d'une requete terminee n'interessent plus personne, et les
    /// garder ferait remettre a la requete suivante des paquets qui ne lui
    /// sont pas destines.
    pub fn desabonne(&mut self) {
        self.abonnee = false;
        self.vide();
    }

    pub fn abonnee(&self) -> bool {
        self.abonnee
    }

    fn vide(&mut self) {
        self.tete = 0;
        self.occupation = 0;
    }

    /// Depose une copie d'une trame. Rend `false` si elle est perdue.
    ///
    /// # La perte est comptee, jamais silencieuse
    ///
    /// Une file pleine signifie que le consommateur ne lit pas assez vite.
    /// C'est une information : elle distingue « smoltcp n'a rien recu » de
    /// « smoltcp n'a pas lu ce qu'on lui a donne ».
    pub fn pose(&mut self, trame: &[u8]) -> bool {
        if !self.abonnee {
            return false;
        }
        if trame.len() > TAILLE {
            self.compteurs.perdues_taille = self.compteurs.perdues_taille.saturating_add(1);
            return false;
        }
        if self.occupation >= TRAMES {
            self.compteurs.perdues_pleine = self.compteurs.perdues_pleine.saturating_add(1);
            return false;
        }
        let place = (self.tete + self.occupation) % TRAMES;
        self.tampons[place][..trame.len()].copy_from_slice(trame);
        self.longueurs[place] = trame.len();
        self.occupation += 1;
        self.compteurs.posees = self.compteurs.posees.saturating_add(1);
        if self.occupation > self.compteurs.occupation_max {
            self.compteurs.occupation_max = self.occupation;
        }
        true
    }

    /// Retire la plus ancienne trame dans `sortie`. Rend sa longueur.
    pub fn retire(&mut self, sortie: &mut [u8]) -> Option<usize> {
        if self.occupation == 0 {
            return None;
        }
        let place = self.tete;
        let longueur = self.longueurs[place].min(sortie.len());
        sortie[..longueur].copy_from_slice(&self.tampons[place][..longueur]);
        self.tete = (self.tete + 1) % TRAMES;
        self.occupation -= 1;
        self.compteurs.retirees = self.compteurs.retirees.saturating_add(1);
        Some(longueur)
    }

    pub fn occupation(&self) -> usize {
        self.occupation
    }

    pub fn compteurs(&self) -> Compteurs {
        self.compteurs
    }
}
