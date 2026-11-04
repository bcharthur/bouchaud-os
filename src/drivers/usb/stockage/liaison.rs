//! Le volume USB et LE support qui le porte (BOUCHAUD_USB_LIAISON_SUPPORT_V1).
//!
//! Pur : aucun registre, aucun verrou. `tools/platform/test_usb_liaison.rs`
//! inclut ce fichier tel quel.
//!
//! # Le defaut que ce module ferme
//!
//! Le pilote publiait l'etat du volume USB dans des atomiques GLOBALES --
//! geometrie et capacite de vidange --, que CHAQUE support pret reecrivait,
//! alors que les entrees-sorties allaient au « premier support pret ». Avec
//! deux cles :
//!
//! * la sonde de la seconde decidait de la promesse de durabilite de la
//!   premiere (dans les deux sens : une promesse retiree a tort, ou -- pire --
//!   accordee a tort) ;
//! * la geometrie publiee pouvait etre celle d'une cle, les blocs lus ceux de
//!   l'autre ;
//! * au debranchement de la premiere, les entrees-sorties basculaient EN
//!   SILENCE sur la seconde, avec l'ancienne promesse.
//!
//! # La regle
//!
//! Chaque support recoit au montage un NUMERO unique depuis le demarrage,
//! jamais reutilise (un `slot_id` xHCI, lui, est recycle). Le volume est LIE a
//! un numero : le premier support pret. Identification, geometrie, lecture,
//! ecriture et vidange designent ce numero, et lui seul ; la capacite de
//! vidange est celle que CE support a prouvee.
//!
//! Le support lie retire, le volume devient ORPHELIN pour le reste du
//! demarrage : plus de geometrie, plus d'entree-sortie, plus aucune promesse.
//! Une cle rebranchee -- la meme ou une autre, on ne peut pas le savoir ici --
//! recoit un nouveau numero et n'est PAS reliee : les couches au-dessus (table
//! de partitions, persistance, A/B) ont lu l'ancien support, et leur rendre un
//! autre support sous le meme volume serait ecrire sur un disque qu'elles ne
//! connaissent pas.

/// Identite d'un support depuis le demarrage. Zero ne designe aucun support.
pub type Numero = u32;

/// Etat du volume USB, publie en un seul mot pour un lecteur sans verrou.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Liaison {
    /// Support lie, 0 si aucun.
    pub lie: Numero,
    /// Le support lie a termine `SYNCHRONIZE CACHE` et ne l'a pas refuse
    /// depuis.
    pub vidange_reelle: bool,
    /// Le support lie a ete retire : le volume ne designe plus rien.
    pub orphelin: bool,
}

/// Ce que devient un support qui vient d'etre pret.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Il porte desormais le volume.
    Lie,
    /// Le volume est deja porte par un autre : ce support n'est pas expose,
    /// et rien de ce qu'il dit ne change le volume.
    DejaLie(Numero),
    /// Le volume a perdu son support pendant ce demarrage : il n'est plus
    /// relie a aucun autre.
    VolumeOrphelin,
}

const BIT_VIDANGE: u64 = 1 << 32;
const BIT_ORPHELIN: u64 = 1 << 33;

impl Liaison {
    pub const NEUVE: Liaison = Liaison { lie: 0, vidange_reelle: false, orphelin: false };

    pub fn empaquete(self) -> u64 {
        self.lie as u64
            | if self.vidange_reelle { BIT_VIDANGE } else { 0 }
            | if self.orphelin { BIT_ORPHELIN } else { 0 }
    }

    pub fn depaquete(mot: u64) -> Self {
        Liaison {
            lie: mot as u32,
            vidange_reelle: mot & BIT_VIDANGE != 0,
            orphelin: mot & BIT_ORPHELIN != 0,
        }
    }

    /// Le support `numero` est pret ; `vidange_sondee` dit si CE support a
    /// termine la sonde `SYNCHRONIZE CACHE`.
    pub fn support_pret(self, numero: Numero, vidange_sondee: bool) -> (Self, Decision) {
        if numero == 0 {
            return (self, Decision::DejaLie(self.lie));
        }
        if self.orphelin {
            return (self, Decision::VolumeOrphelin);
        }
        if self.lie != 0 {
            return (self, Decision::DejaLie(self.lie));
        }
        (Liaison { lie: numero, vidange_reelle: vidange_sondee, orphelin: false }, Decision::Lie)
    }

    /// Le support `numero` vient d'etre retire.
    pub fn support_retire(self, numero: Numero) -> Self {
        if numero == 0 || numero != self.lie || self.orphelin {
            return self;
        }
        Liaison { lie: self.lie, vidange_reelle: false, orphelin: true }
    }

    /// Le support `numero` a refuse une vidange. Seul le support lie peut
    /// retirer la promesse du volume ; un autre n'en a aucune a retirer.
    pub fn vidange_refusee(self, numero: Numero) -> Self {
        if numero != 0 && numero == self.lie {
            Liaison { vidange_reelle: false, ..self }
        } else {
            self
        }
    }

    /// Le support que le volume designe, s'il en designe un.
    pub fn cible(self) -> Option<Numero> {
        (self.lie != 0 && !self.orphelin).then_some(self.lie)
    }

    /// Le volume promet-il une barriere de durabilite ?
    pub fn promet_la_vidange(self) -> bool {
        self.vidange_reelle && self.cible().is_some()
    }

    /// Une vidange REUSSIE sur `numero` vaut-elle pour le volume ? Seulement si
    /// `numero` est encore le support lie et qu'il avait prouve la vidange :
    /// une reussite sur un support qui n'est plus celui du volume ne garantit
    /// rien des ecritures du volume.
    pub fn vidange_valide(self, numero: Numero) -> bool {
        self.promet_la_vidange() && numero == self.lie
    }
}

// ---------------------------------------------------------------------------
// Un refus de commande, et ce qu'il faut faire avant la suivante
// ---------------------------------------------------------------------------

/// Origine d'un echec de `SYNCHRONIZE CACHE`, telle que le transport la rend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refus {
    /// Aucune commande n'est partie (injection de banc, support absent).
    SansCommande,
    /// La commande a ete REFUSEE : CSW statut 1, « Command Failed ». Le
    /// transport est sain ; le support a une condition de sens en attente.
    Commande,
    /// Le transport lui-meme a failli (echeance, STALL non recupere, erreur de
    /// phase, CSW invalide). L'etat du transport le dit deja.
    Transport,
}

/// Classe un motif d'echec de `bot_commande`.
pub fn classe_refus(motif: &str) -> Refus {
    match motif {
        "bot-commande-echouee" => Refus::Commande,
        "vidange-refus-injecte" | "absent" | "slot" | "peripherique" | "pilote-occupe" => {
            Refus::SansCommande
        }
        _ => Refus::Transport,
    }
}

/// Ce que le pilote doit faire, SOUS LE MEME VERROU, avant de rendre la main.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Traitement {
    /// Reprise Bulk-Only (reinitialisation de classe, deblocage des deux
    /// points) avant toute autre commande.
    pub reprise: bool,
    /// `REQUEST SENSE` : lire la condition en attente l'efface. Certaines cles
    /// refusent sinon TOUTE commande suivante, lectures comprises.
    pub demande_sens: bool,
}

pub fn traitement(refus: Refus) -> Traitement {
    match refus {
        Refus::SansCommande => Traitement { reprise: false, demande_sens: false },
        Refus::Commande => Traitement { reprise: false, demande_sens: true },
        // Apres la reprise, le sens est demande aussi : une reinitialisation
        // Bulk-Only ne promet pas d'effacer la condition de l'unite logique.
        Refus::Transport => Traitement { reprise: true, demande_sens: true },
    }
}

/// Donnees de sens (SPC-4, 4.5), formats fixe et descripteur.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sens {
    pub cle: u8,
    pub asc: u8,
    pub ascq: u8,
}

/// `ILLEGAL REQUEST` : le support ne connait pas, ou n'accepte pas, la commande.
pub const CLE_REQUETE_ILLEGALE: u8 = 0x5;

pub fn decode_sens(entree: &[u8]) -> Option<Sens> {
    let code = *entree.first()? & 0x7f;
    match code {
        0x70 | 0x71 => {
            if entree.len() < 14 {
                return None;
            }
            Some(Sens { cle: entree[2] & 0x0f, asc: entree[12], ascq: entree[13] })
        }
        0x72 | 0x73 => {
            if entree.len() < 4 {
                return None;
            }
            Some(Sens { cle: entree[1] & 0x0f, asc: entree[2], ascq: entree[3] })
        }
        _ => None,
    }
}

impl Sens {
    pub fn nom_cle(self) -> &'static str {
        match self.cle {
            0x0 => "aucune",
            0x1 => "recupere",
            0x2 => "pas-pret",
            0x3 => "erreur-support",
            0x4 => "erreur-materiel",
            0x5 => "requete-illegale",
            0x6 => "attention-unite",
            0x7 => "protege",
            0xb => "commande-abandonnee",
            _ => "autre",
        }
    }
}
