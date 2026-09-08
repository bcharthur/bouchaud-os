//! Ce qu'il faut CALCULER pour atteindre un peripherique derriere un
//! concentrateur, sans toucher un seul registre.
//!
//! # Pourquoi ce fichier existe a part
//!
//! Un clavier branche derriere un concentrateur ne repond pas si UN champ de
//! bits est faux. Il n'y a pas de message d'erreur : le controleur adresse un
//! peripherique qui n'est pas la, la commande echoue ou -- pire -- reussit sur
//! le mauvais, et le journal dit « adresse ok ». On ne le decouvre qu'en
//! branchant un vrai concentrateur.
//!
//! Ces champs sont donc calcules ici, ou une machine peut les relire : chaine
//! de route, contexte de slot, descripteur de concentrateur, etat d'un port.
//! Le pilote, lui, ne fait plus qu'ecrire ce qu'on lui rend.
//!
//! # Ce que ce fichier ne fait pas
//!
//! Il ne connait ni le materiel, ni le temps, ni le journal. Toute fonction
//! qui aurait besoin de l'un des trois n'a rien a faire ici.

#![allow(dead_code)]

// ---------------------------------------------------------------------------
// Identifiants de vitesse xHCI (valeurs par defaut, xHCI 1.2 tableau 7-13).
// ---------------------------------------------------------------------------

/// Full Speed, 12 Mbit/s.
pub const VITESSE_PLEINE: u8 = 1;
/// Low Speed, 1,5 Mbit/s -- la plupart des claviers et souris filaires.
pub const VITESSE_BASSE: u8 = 2;
/// High Speed, 480 Mbit/s.
pub const VITESSE_HAUTE: u8 = 3;
/// SuperSpeed, 5 Gbit/s.
pub const VITESSE_SUPER: u8 = 4;
/// SuperSpeedPlus, 10 Gbit/s.
pub const VITESSE_SUPER_PLUS: u8 = 5;

/// Une chaine de route xHCI compte CINQ etages de quatre bits.
///
/// Ce n'est pas une limite qu'on s'impose : le champ fait vingt bits. Un
/// sixieme concentrateur en cascade n'est pas adressable, quel que soit le
/// code qu'on ecrive.
pub const ETAGES_MAX: usize = 5;

/// Classe d'un concentrateur, dans le descripteur de peripherique.
pub const CLASSE_CONCENTRATEUR: u8 = 0x09;

/// Type du descripteur d'un concentrateur USB 2.0.
pub const DESCRIPTEUR_CONCENTRATEUR: u8 = 0x29;
/// Type du descripteur d'un concentrateur SuperSpeed.
pub const DESCRIPTEUR_CONCENTRATEUR_SUPER: u8 = 0x2a;

// ---------------------------------------------------------------------------
// La chaine de route.
// ---------------------------------------------------------------------------

/// La chaine de route d'un peripherique branche au port `port` d'un
/// concentrateur situe a la profondeur `profondeur`.
///
/// # Les deux pieges de ce calcul
///
/// **Le decalage suit la profondeur du PARENT.** Un peripherique branche
/// directement sur le concentrateur racine porte son numero de port dans les
/// bits 0 a 3 ; un peripherique branche derriere lui, dans les bits 4 a 7.
/// Se tromper d'etage designe un autre port, sur lequel il y a peut-etre un
/// autre peripherique -- et l'adressage REUSSIT alors, sur le mauvais.
///
/// **Un port au-dela de quinze se code quinze.** Le champ fait quatre bits.
/// La specification USB 3 le dit explicitement : au-dela, on sature. Masquer
/// (`port & 0xf`) au lieu de saturer designerait le port 1 pour le port 17.
///
/// Rend `None` au-dela de cinq etages : le champ est plein, et rien ne peut
/// designer ce peripherique.
pub const fn route_enfant(route: u32, profondeur: usize, port: u8) -> Option<u32> {
    if profondeur >= ETAGES_MAX {
        return None;
    }
    if port == 0 {
        return None;
    }
    let code = if port > 15 { 15u32 } else { port as u32 };
    Some(route | (code << (4 * profondeur)))
}

// ---------------------------------------------------------------------------
// Le contexte de slot.
// ---------------------------------------------------------------------------

/// Mot 0 du contexte de slot.
///
/// Route (0-19), vitesse (20-23), Multi-TT (25), Hub (26), entrees (27-31).
///
/// Le bit `Hub` n'est pas decoratif : sans lui, le controleur refuse
/// d'adresser quoi que ce soit derriere ce peripherique. C'est la difference
/// entre « le concentrateur est enumere » et « le concentrateur sert ».
pub const fn slot_dw0(
    route: u32,
    vitesse: u8,
    entrees: u8,
    concentrateur: bool,
    multi_tt: bool,
) -> u32 {
    (route & 0x000f_ffff)
        | (((vitesse & 0xf) as u32) << 20)
        | (if multi_tt { 1 << 25 } else { 0 })
        | (if concentrateur { 1 << 26 } else { 0 })
        | (((entrees & 0x1f) as u32) << 27)
}

/// Mot 1 du contexte de slot : latence de sortie (0-15), port RACINE (16-23),
/// nombre de ports (24-31).
///
/// « Port racine » compte : c'est toujours le port du concentrateur RACINE au
/// pied de l'arbre, jamais le port du concentrateur intermediaire. Le port
/// intermediaire, lui, est dans la chaine de route.
pub const fn slot_dw1(latence_sortie: u16, port_racine: u8, nombre_de_ports: u8) -> u32 {
    (latence_sortie as u32)
        | ((port_racine as u32) << 16)
        | ((nombre_de_ports as u32) << 24)
}

/// Mot 2 du contexte de slot : slot du transactionneur (0-7), port du
/// transactionneur (8-15), temps de reflexion (16-17), interrupteur (22-31).
pub const fn slot_dw2(tt_slot: u8, tt_port: u8, temps_reflexion: u8) -> u32 {
    (tt_slot as u32) | ((tt_port as u32) << 8) | (((temps_reflexion & 0x3) as u32) << 16)
}

/// Un peripherique lent derriere un concentrateur rapide a besoin d'un
/// TRANSACTIONNEUR.
///
/// Un bus haute vitesse ne transporte pas directement une transaction basse ou
/// pleine vitesse : le concentrateur haute vitesse la traduit, et le
/// controleur doit savoir LEQUEL, et sur quel port. Sans ces deux champs, le
/// peripherique est adresse et ne repond a rien -- ce qui est exactement le
/// symptome qu'on cherche a fermer.
///
/// La quasi-totalite des claviers et souris filaires sont basse vitesse : ce
/// cas n'est pas l'exception, c'est le cas courant.
pub const fn requiert_transactionneur(vitesse_parent: u8, vitesse_enfant: u8) -> bool {
    vitesse_parent >= VITESSE_HAUTE
        && (vitesse_enfant == VITESSE_BASSE || vitesse_enfant == VITESSE_PLEINE)
}

// ---------------------------------------------------------------------------
// Le descripteur d'un concentrateur.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Descripteur {
    /// Nombre de ports en aval.
    pub ports: u8,
    /// Temps de reflexion du transactionneur, tel quel pour le contexte de
    /// slot (0 = 8 temps de bit, 1 = 16, 2 = 24, 3 = 32).
    pub temps_reflexion: u8,
    /// Delai a respecter apres avoir alimente un port, en millisecondes.
    pub delai_alimentation_ms: u16,
}

/// Lit le descripteur d'un concentrateur, USB 2.0 (`0x29`) ou SuperSpeed
/// (`0x2a`).
///
/// Les deux dispositions different apres le sixieme octet, et coincident sur
/// tout ce dont on a besoin : nombre de ports en 2, caracteristiques en 3-4,
/// delai d'alimentation en 5.
///
/// # Le delai d'alimentation
///
/// `bPwrOn2PwrGood` se compte en unites de DEUX millisecondes. Le prendre pour
/// des millisecondes interroge le port avant qu'il ne soit alimente : il se
/// dit vide, et le peripherique qui y est branche n'existe pas. C'est une
/// erreur qui ne se voit que sur les concentrateurs lents a s'allumer -- donc
/// jamais sous emulation.
pub fn descripteur(octets: &[u8]) -> Option<Descripteur> {
    if octets.len() < 6 {
        return None;
    }
    let longueur = octets[0] as usize;
    let genre = octets[1];
    if genre != DESCRIPTEUR_CONCENTRATEUR && genre != DESCRIPTEUR_CONCENTRATEUR_SUPER {
        return None;
    }
    // LA LONGUEUR ANNONCEE EST PLUS GRANDE QUE CE QU'ON A LU, ET C'EST NORMAL.
    //
    // Un descripteur de concentrateur se termine par deux tableaux dont la
    // taille depend du nombre de ports : `bDescLength` vaut 13 pour huit
    // ports, 9 pour un seul. On n'en lit que la partie fixe -- c'est tout ce
    // dont on se sert -- donc exiger que la longueur ANNONCEE tienne dans ce
    // qu'on a LU rejette tous les concentrateurs de plus d'un port.
    //
    // C'est exactement ce qui est arrive : le concentrateur etait reconnu,
    // configure, et sa traversee refusee sur ce seul motif.
    //
    // Ce qu'il faut verifier, c'est que la partie FIXE est complete -- sept
    // octets, USB 2.0 §11.23.2.1 -- et que nos six premiers octets sont la.
    // On ne lit rien au-dela.
    if longueur < 7 {
        return None;
    }
    let ports = octets[2];
    if ports == 0 {
        // Un concentrateur sans port n'est pas un concentrateur. Le prendre au
        // mot ferait boucler la traversee sur rien.
        return None;
    }
    let caracteristiques = u16::from_le_bytes([octets[3], octets[4]]);
    Some(Descripteur {
        ports,
        temps_reflexion: ((caracteristiques >> 5) & 0x3) as u8,
        // Sature : un octet vaut au plus 255, donc 510 ms.
        delai_alimentation_ms: (octets[5] as u16).saturating_mul(2),
    })
}

// ---------------------------------------------------------------------------
// L'etat d'un port de concentrateur.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EtatPort {
    pub connecte: bool,
    pub active: bool,
    pub en_reinitialisation: bool,
    pub alimente: bool,
    /// Identifiant de vitesse xHCI, ou zero tant que le port n'est pas active.
    pub vitesse: u8,
    /// Bits de changement, a effacer un par un.
    pub changements: u16,
}

/// Lit les quatre octets rendus par `GET_STATUS` sur un port.
///
/// # Deux dispositions, et c'est la source d'erreur
///
/// Un concentrateur USB 2.0 code la vitesse par deux bits SEPARES : bit 9
/// « basse vitesse », bit 10 « haute vitesse », aucun des deux voulant dire
/// « pleine vitesse ». Un concentrateur SuperSpeed code un IDENTIFIANT sur
/// trois bits au meme endroit, et y place l'alimentation ailleurs.
///
/// Lire l'un avec la disposition de l'autre rend une vitesse fausse -- et une
/// vitesse fausse fait programmer une taille de paquet fausse, donc un
/// peripherique qui ne repond jamais.
pub fn etat_port(octets: &[u8], superspeed: bool) -> Option<EtatPort> {
    if octets.len() < 4 {
        return None;
    }
    let etat = u16::from_le_bytes([octets[0], octets[1]]);
    let changements = u16::from_le_bytes([octets[2], octets[3]]);
    let connecte = etat & (1 << 0) != 0;
    let active = etat & (1 << 1) != 0;
    let en_reinitialisation = etat & (1 << 4) != 0;
    let (alimente, vitesse) = if superspeed {
        let identifiant = ((etat >> 10) & 0x7) as u8;
        (
            etat & (1 << 9) != 0,
            if !active {
                0
            } else if identifiant == 0 {
                VITESSE_SUPER
            } else {
                VITESSE_SUPER_PLUS
            },
        )
    } else {
        (
            etat & (1 << 8) != 0,
            if !active {
                0
            } else if etat & (1 << 9) != 0 {
                VITESSE_BASSE
            } else if etat & (1 << 10) != 0 {
                VITESSE_HAUTE
            } else {
                VITESSE_PLEINE
            },
        )
    };
    Some(EtatPort {
        connecte,
        active,
        en_reinitialisation,
        alimente,
        vitesse,
        changements,
    })
}

// ---------------------------------------------------------------------------
// Les requetes de classe.
// ---------------------------------------------------------------------------

/// Fonctionnalites d'un port, telles que les nomme USB 2.0 tableau 11-17.
pub const PORT_ACTIVATION: u16 = 1;
pub const PORT_REINITIALISATION: u16 = 4;
pub const PORT_ALIMENTATION: u16 = 8;
pub const C_PORT_CONNEXION: u16 = 16;
pub const C_PORT_ACTIVATION: u16 = 17;
pub const C_PORT_SUSPENSION: u16 = 18;
pub const C_PORT_SURINTENSITE: u16 = 19;
pub const C_PORT_REINITIALISATION: u16 = 20;

/// Les bits de changement, dans l'ordre des fonctionnalites qui les effacent.
///
/// Un changement qu'on n'efface pas reste pose. Le concentrateur continue de
/// le signaler, la traversee le relit a chaque tour, et on rebranche
/// indefiniment le meme peripherique.
pub const CHANGEMENTS: [(u16, u16); 5] = [
    (1 << 0, C_PORT_CONNEXION),
    (1 << 1, C_PORT_ACTIVATION),
    (1 << 2, C_PORT_SUSPENSION),
    (1 << 3, C_PORT_SURINTENSITE),
    (1 << 4, C_PORT_REINITIALISATION),
];

/// `bmRequestType` d'une requete de classe adressee a un PORT.
///
/// Le destinataire est « autre » (valeur 3), pas « peripherique ». Adresser la
/// requete au peripherique la fait porter sur le concentrateur entier : un
/// `SET_FEATURE(RESET)` reinitialiserait le concentrateur lui-meme, et avec
/// lui tout ce qui est branche dessus.
pub const CLASSE_VERS_PORT_ECRITURE: u8 = 0x23;
pub const CLASSE_VERS_PORT_LECTURE: u8 = 0xa3;
/// `bmRequestType` d'une lecture de descripteur adressee au CONCENTRATEUR.
pub const CLASSE_VERS_CONCENTRATEUR_LECTURE: u8 = 0xa0;

/// Le paquet `SETUP` d'une requete standard ou de classe, dans l'ordre du fil.
pub const fn paquet_setup(
    genre: u8,
    requete: u8,
    valeur: u16,
    index: u16,
    longueur: u16,
) -> u64 {
    (genre as u64)
        | ((requete as u64) << 8)
        | ((valeur as u64) << 16)
        | ((index as u64) << 32)
        | ((longueur as u64) << 48)
}

/// `SET_FEATURE` sur un port.
pub const fn requete_pose_port(fonctionnalite: u16, port: u8) -> u64 {
    paquet_setup(CLASSE_VERS_PORT_ECRITURE, 3, fonctionnalite, port as u16, 0)
}

/// `CLEAR_FEATURE` sur un port.
pub const fn requete_efface_port(fonctionnalite: u16, port: u8) -> u64 {
    paquet_setup(CLASSE_VERS_PORT_ECRITURE, 1, fonctionnalite, port as u16, 0)
}

/// `GET_STATUS` sur un port : quatre octets.
pub const fn requete_etat_port(port: u8) -> u64 {
    paquet_setup(CLASSE_VERS_PORT_LECTURE, 0, 0, port as u16, 4)
}

/// `GET_DESCRIPTOR` du concentrateur lui-meme.
pub const fn requete_descripteur(superspeed: bool, longueur: u16) -> u64 {
    let genre = if superspeed {
        DESCRIPTEUR_CONCENTRATEUR_SUPER
    } else {
        DESCRIPTEUR_CONCENTRATEUR
    };
    paquet_setup(
        CLASSE_VERS_CONCENTRATEUR_LECTURE,
        6,
        (genre as u16) << 8,
        0,
        longueur,
    )
}
