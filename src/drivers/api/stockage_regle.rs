//! BOUCHAUD_CLE_USB_TEST_V1 -- la regle, PURE : quelle commande de stockage
//! peut partir vers quel support.
//!
//! Aucun acces au noyau, aucune allocation : compile aussi sur l'hote par
//! `tools/securite/test_stockage_regle.rs` (lance par
//! `tools/ci/run_host_tests.sh`). Le noyau l'appelle depuis
//! `drivers::api::ecriture_interne`, aux trois seuls points ou une commande
//! quitte la machine vers un support :
//!   - ATA : l'ecriture du registre de commande (`ata::emet_commande`) ;
//!   - NVMe : l'ecriture d'une entree de file de soumission (`File::pose`) ;
//!   - USB stockage de masse : le codage d'un CBW (`bot_commande_une_fois`,
//!     et la boite noire `blackbox_bot_en_place`).
//!
//! # Le modele de confiance
//!
//! Dans une image `cle-usb-test`, une commande n'est permise que si elle est
//! sur une LISTE BLANCHE de commandes qui ne modifient pas le support
//! (lecture, identification, mise en place des files). Tout le reste --
//! ecriture, vidange, formatage, effacement, TRIM, micrologiciel, passage ATA
//! par SCSI, commandes inconnues -- est refuse :
//!   - toujours sur un disque ATA ou NVMe : ce sont les disques INTERNES ;
//!   - sur un stockage USB, sauf s'il est le support de demarrage AUTORISE.
//!
//! Le support autorise n'est pas reconnu au nom de son pilote : il porte, dans
//! l'en-tete GPT primaire (signature ET somme de controle verifiees), le GUID
//! de disque tire au hasard a la construction de l'image et grave dans le
//! noyau. Le moindre doute -- en-tete illisible, somme fausse, GUID different,
//! noyau sans GUID, DEUX supports qui portent le meme -- rend le support non
//! autorise. Refuser est le seul repli : un essai sur une machine qui porte
//! deja un systeme ne doit jamais pouvoir l'abimer.
//!
//! Hors image `cle-usb-test`, tout est permis : la regle ne change rien au
//! comportement d'une image standard.

/// Le transport d'un support.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Support {
    Ata,
    Nvme,
    Usb,
}

impl Support {
    pub const fn nom(self) -> &'static str {
        match self {
            Support::Ata => "ata",
            Support::Nvme => "nvme",
            Support::Usb => "usb",
        }
    }
}

/// Ce que l'on sait d'un stockage USB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Identite {
    /// Pas (encore) d'en-tete GPT lisible et valide : etat initial.
    Inconnue,
    /// En-tete GPT valide, GUID de disque different de celui de l'image.
    Etrangere,
    /// En-tete GPT valide portant le GUID de l'image.
    Autorisee,
}

impl Identite {
    pub const fn nom(self) -> &'static str {
        match self {
            Identite::Inconnue => "inconnue",
            Identite::Etrangere => "etrangere",
            Identite::Autorisee => "autorisee",
        }
    }
}

/// Pourquoi une commande est refusee.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Raison {
    /// Disque ATA ou NVMe : interne, jamais modifiable dans cette image.
    DisqueInterne,
    /// Stockage USB qui n'est pas le support de demarrage autorise.
    SupportNonAutorise,
    /// Stockage USB dont l'identite n'est pas etablie.
    IdentiteIncertaine,
    /// Deux supports se presentent comme le support autorise.
    IdentiteAmbigue,
    /// Commande hors liste blanche, meme sur le support autorise.
    CommandeInterdite,
}

impl Raison {
    pub const fn nom(self) -> &'static str {
        match self {
            Raison::DisqueInterne => "disque-interne",
            Raison::SupportNonAutorise => "support-non-autorise",
            Raison::IdentiteIncertaine => "identite-incertaine",
            Raison::IdentiteAmbigue => "identite-ambigue",
            Raison::CommandeInterdite => "commande-interdite",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Permis,
    Refuse(Raison),
}

impl Verdict {
    pub const fn permis(self) -> bool {
        matches!(self, Verdict::Permis)
    }
}

/// Classe d'une commande, une fois decodee.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Classe {
    /// Ne modifie pas le support (lecture, identification, files...).
    SansEffet,
    /// Ecriture de donnees ou vidange du cache : permise sur le seul support
    /// autorise.
    Ecriture,
    /// Tout le reste : refuse dans l'image `cle-usb-test`, quel que soit le
    /// support (formatage, effacement, TRIM, micrologiciel, passage ATA...).
    Interdite,
}

// --- ATA ---------------------------------------------------------------------

/// Classe d'une commande ATA (registre de commande).
pub const fn classe_ata(commande: u8) -> Classe {
    match commande {
        0xEC // IDENTIFY DEVICE
        | 0xA1 // IDENTIFY PACKET DEVICE
        | 0x20 | 0x21 // READ SECTORS (avec/sans reprise)
        | 0x24 // READ SECTORS EXT
        | 0xC8 | 0xC9 // READ DMA
        | 0x25 // READ DMA EXT
        | 0xC4 | 0x29 // READ MULTIPLE (EXT)
        | 0x40 | 0x41 | 0x42 // READ VERIFY SECTORS (EXT)
        | 0xE5 // CHECK POWER MODE
        => Classe::SansEffet,
        0x30 | 0x31 // WRITE SECTORS
        | 0x34 // WRITE SECTORS EXT
        | 0xCA | 0xCB // WRITE DMA
        | 0x35 // WRITE DMA EXT
        | 0xC5 | 0x39 // WRITE MULTIPLE (EXT)
        | 0x3D // WRITE DMA FUA EXT
        | 0xE7 | 0xEA // FLUSH CACHE (EXT)
        => Classe::Ecriture,
        // SECURITY ERASE, TRIM (DATA SET MANAGEMENT), DOWNLOAD MICROCODE,
        // SET FEATURES, SET MAX, SANITIZE, et toute commande inconnue.
        _ => Classe::Interdite,
    }
}

// --- NVMe --------------------------------------------------------------------

/// Classe d'une entree de file de soumission NVMe (`mots[0]` : opcode dans
/// les huit bits bas ; `mots[10]` : CDW10, l'attribut pour Set Features).
pub const fn classe_nvme(admin: bool, mots: &[u32; 16]) -> Classe {
    let opcode = (mots[0] & 0xFF) as u8;
    if admin {
        match opcode {
            0x00 // Delete I/O SQ
            | 0x01 // Create I/O SQ
            | 0x02 // Get Log Page
            | 0x04 // Delete I/O CQ
            | 0x05 // Create I/O CQ
            | 0x06 // Identify
            | 0x08 // Abort
            | 0x0A // Get Features
            | 0x0C // Asynchronous Event Request
            => Classe::SansEffet,
            // Set Features : le seul attribut permis est le nombre de files
            // (FID 0x07), sans effet sur le support. Cache d'ecriture,
            // gestion d'energie, attributs persistants... : refuses.
            0x09 if mots[10] & 0xFF == 0x07 && mots[10] & (1 << 31) == 0 => Classe::SansEffet,
            // Namespace Management/Attachment, Firmware Download/Commit,
            // Format NVM, Sanitize, Security Send, Device Self-test,
            // Set Features autres, opcodes inconnus et propres au fabricant.
            _ => Classe::Interdite,
        }
    } else {
        match opcode {
            0x02 // Read
            => Classe::SansEffet,
            0x00 // Flush
            | 0x01 // Write
            => Classe::Ecriture,
            // Write Uncorrectable, Compare (inutile), Write Zeroes, Dataset
            // Management (TRIM), Reservation *, Copy, Zone *, inconnus.
            _ => Classe::Interdite,
        }
    }
}

// --- SCSI (USB stockage de masse) ---------------------------------------------

/// Classe d'un bloc de commande SCSI.
pub const fn classe_scsi(cdb: &[u8]) -> Classe {
    if cdb.is_empty() {
        return Classe::Interdite;
    }
    match cdb[0] {
        0x00 // TEST UNIT READY
        | 0x03 // REQUEST SENSE
        | 0x12 // INQUIRY
        | 0x1A | 0x5A // MODE SENSE (6/10)
        | 0x23 // READ FORMAT CAPACITIES
        | 0x25 // READ CAPACITY (10)
        | 0x28 | 0xA8 | 0x88 // READ (10/12/16)
        => Classe::SansEffet,
        // SERVICE ACTION IN (16) : READ CAPACITY (16) seulement.
        0x9E if cdb.len() > 1 && cdb[1] & 0x1F == 0x10 => Classe::SansEffet,
        0x2A | 0xAA | 0x8A // WRITE (10/12/16)
        | 0x35 | 0x91 // SYNCHRONIZE CACHE (10/16)
        => Classe::Ecriture,
        // FORMAT UNIT, WRITE SAME, UNMAP, WRITE AND VERIFY, WRITE BUFFER,
        // MODE SELECT, START STOP UNIT, ATA PASS-THROUGH (12/16 : une
        // ecriture ATA deguisee vers un pont USB-SATA), SECURITY PROTOCOL
        // OUT, SANITIZE, PREVENT ALLOW MEDIUM REMOVAL, inconnus.
        _ => Classe::Interdite,
    }
}

// --- Le verdict ---------------------------------------------------------------

/// Le verdict pour une commande deja classee.
///
/// `cle` : l'image est-elle `cle-usb-test` ? `ambigu` : deux supports ont
/// deja presente le GUID autorise depuis le demarrage.
pub const fn verdict(cle: bool, support: Support, identite: Identite, ambigu: bool, classe: Classe) -> Verdict {
    if !cle {
        return Verdict::Permis;
    }
    match classe {
        Classe::SansEffet => Verdict::Permis,
        Classe::Interdite => Verdict::Refuse(match support {
            Support::Ata | Support::Nvme => Raison::DisqueInterne,
            Support::Usb => Raison::CommandeInterdite,
        }),
        Classe::Ecriture => match support {
            Support::Ata | Support::Nvme => Verdict::Refuse(Raison::DisqueInterne),
            Support::Usb => match identite {
                Identite::Autorisee if ambigu => Verdict::Refuse(Raison::IdentiteAmbigue),
                Identite::Autorisee => Verdict::Permis,
                Identite::Etrangere => Verdict::Refuse(Raison::SupportNonAutorise),
                Identite::Inconnue => Verdict::Refuse(Raison::IdentiteIncertaine),
            },
        },
    }
}

// --- L'identite du support autorise --------------------------------------------

/// Le GUID de disque autorise, tel que grave a la construction : 32 chiffres
/// hexadecimaux, les seize octets DANS L'ORDRE OU ILS SONT ECRITS SUR LE
/// DISQUE (octets 56..72 de l'en-tete GPT) -- aucune conversion d'ordre mixte
/// a faire ni a rater. Rend `None` pour un texte absent, mal forme, ou nul (un
/// GUID nul n'identifie rien).
pub const fn guid_autorise(texte: Option<&str>) -> Option<[u8; 16]> {
    let Some(texte) = texte else { return None };
    let o = texte.as_bytes();
    if o.len() != 32 {
        return None;
    }
    let mut guid = [0u8; 16];
    let mut i = 0;
    let mut non_nul = false;
    while i < 16 {
        let (Some(h), Some(b)) = (chiffre(o[2 * i]), chiffre(o[2 * i + 1])) else { return None };
        guid[i] = (h << 4) | b;
        non_nul |= guid[i] != 0;
        i += 1;
    }
    if non_nul { Some(guid) } else { None }
}

const fn chiffre(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

pub const TAILLE_ENTETE_GPT: usize = 92;

/// CRC32 IEEE (celui de GPT).
pub fn crc32(donnees: &[u8]) -> u32 {
    let mut reste = 0xFFFF_FFFFu32;
    for octet in donnees {
        reste ^= *octet as u32;
        for _ in 0..8 {
            let bas = reste & 1;
            reste >>= 1;
            if bas != 0 {
                reste ^= 0xEDB8_8320;
            }
        }
    }
    !reste
}

/// Le GUID de disque d'un en-tete GPT PRIMAIRE (le bloc LBA 1), si la
/// signature, la taille annoncee, la somme de controle et `MyLBA == 1` sont
/// justes. Sans allocation : la somme se calcule sur une copie de pile.
pub fn guid_disque(bloc: &[u8]) -> Option<[u8; 16]> {
    if bloc.len() < TAILLE_ENTETE_GPT || &bloc[0..8] != b"EFI PART" {
        return None;
    }
    let taille = u32::from_le_bytes([bloc[12], bloc[13], bloc[14], bloc[15]]) as usize;
    // La norme borne l'en-tete a la taille d'un bloc ; 512 couvre les
    // supports de 512 octets, et un en-tete plus grand n'a jamais ete vu.
    if taille < TAILLE_ENTETE_GPT || taille > 512 || taille > bloc.len() {
        return None;
    }
    let annoncee = u32::from_le_bytes([bloc[16], bloc[17], bloc[18], bloc[19]]);
    let mut copie = [0u8; 512];
    copie[..taille].copy_from_slice(&bloc[..taille]);
    copie[16..20].fill(0);
    if crc32(&copie[..taille]) != annoncee {
        return None;
    }
    let mon_lba = u64::from_le_bytes([
        bloc[24], bloc[25], bloc[26], bloc[27], bloc[28], bloc[29], bloc[30], bloc[31],
    ]);
    if mon_lba != 1 {
        return None;
    }
    let mut guid = [0u8; 16];
    guid.copy_from_slice(&bloc[56..72]);
    Some(guid)
}

/// L'identite d'un stockage USB d'apres son bloc LBA 1 (`None` : illisible).
pub fn identite(bloc_lba1: Option<&[u8]>, autorise: Option<[u8; 16]>) -> Identite {
    let Some(bloc) = bloc_lba1 else { return Identite::Inconnue };
    let Some(guid) = guid_disque(bloc) else { return Identite::Inconnue };
    match autorise {
        Some(a) if a == guid => Identite::Autorisee,
        // Pas de GUID dans le noyau : rien ne peut etre autorise.
        _ => Identite::Etrangere,
    }
}
