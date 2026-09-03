use super::protocole::{longueur_physique, rogne_degat, Rect, ECHELLE_UNITE};

// Le registre vit dans son propre fichier, mais dans le MEME module : il
// manipule les memes types de charge utile, et `include!` evite d'avoir a
// rendre publique la moitie du contrat pour qu'un sous-module y accede.
include!("composited/etat.rs");

/// "BOCO" -- un flux qui n'est pas le notre est rejete a l'octet pres.
pub const MAGIC: u32 = 0x4f43_4f42;
pub const VERSION: u16 = 1;

/// Tampons par surface. Deux : un affiche, un en cours d'ecriture.
///
/// Le triple tampon viendra ; il se decide sur une mesure de trames manquees,
/// pas sur une intuition, et le format du fil le prevoit deja (`tampon` est un
/// indice, pas un booleen).
pub const TAMPONS: usize = 2;

/// Surfaces suivies simultanement. Borne fixe : le registre est un tableau, et
/// un compositeur qui alloue par surface se fait epuiser par un client hostile.
pub const SURFACES_MAX: usize = 32;

/// Taille de l'en-tete d'un message.
pub const TAILLE_ENTETE: usize = 16;

/// Plus grande charge utile acceptee.
pub const CHARGE_MAX: u32 = 4096;

// --- Messages ----------------------------------------------------------------

#[repr(u16)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Genre {
    // --- client -> composited ---
    /// Demande une surface de la taille LOGIQUE donnee.
    DemandeSurface = 1,
    /// Le client a fini d'ecrire dans un tampon.
    TrameLivree = 2,
    /// Le client abandonne sa surface.
    Detache = 3,

    // --- composited -> client ---
    /// La surface accordee : geometrie physique, echelle, tampons.
    SurfaceAccordee = 0x100,
    /// Un tampon n'est plus affiche : le client peut y ecrire.
    TamponRendu = 0x101,
    /// La geometrie a change.
    Reconfigure = 0x102,
    /// La demande est refusee, avec sa raison.
    Refus = 0x103,
}

impl Genre {
    pub const fn depuis_u16(valeur: u16) -> Option<Genre> {
        Some(match valeur {
            1 => Genre::DemandeSurface,
            2 => Genre::TrameLivree,
            3 => Genre::Detache,
            0x100 => Genre::SurfaceAccordee,
            0x101 => Genre::TamponRendu,
            0x102 => Genre::Reconfigure,
            0x103 => Genre::Refus,
            _ => return None,
        })
    }

    /// Ce message vient-il du client ?
    ///
    /// Un compositeur qui traite un message reserve a ses propres reponses
    /// accepterait qu'un client s'accorde lui-meme une surface.
    pub const fn du_client(self) -> bool {
        (self as u16) < 0x100
    }
}

/// Pourquoi une demande est refusee. Une raison NOMMEE, parce qu'un client qui
/// ne comprend pas son refus reessaie en boucle.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refus {
    /// Le registre est plein.
    PlusDeSurface = 1,
    /// La geometrie demandee est absurde ou trop grande.
    GeometrieInvalide = 2,
    /// Ce client possede deja une surface.
    DejaAttache = 3,
    /// Le tampon annonce n'appartient pas au client.
    TamponNonPossede = 4,
    /// Aucune surface pour ce client.
    Inconnue = 5,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entete {
    pub magic: u32,
    pub version: u16,
    pub genre: u16,
    pub taille_charge: u32,
    pub serie: u32,
}

impl Entete {
    pub const fn neuf(genre: Genre, taille_charge: u32, serie: u32) -> Self {
        Self { magic: MAGIC, version: VERSION, genre: genre as u16, taille_charge, serie }
    }

    pub const fn valide(&self) -> bool {
        self.magic == MAGIC && self.version == VERSION
    }

    pub fn encode(&self) -> [u8; TAILLE_ENTETE] {
        let mut o = [0u8; TAILLE_ENTETE];
        o[0..4].copy_from_slice(&self.magic.to_le_bytes());
        o[4..6].copy_from_slice(&self.version.to_le_bytes());
        o[6..8].copy_from_slice(&self.genre.to_le_bytes());
        o[8..12].copy_from_slice(&self.taille_charge.to_le_bytes());
        o[12..16].copy_from_slice(&self.serie.to_le_bytes());
        o
    }

    pub fn decode(o: &[u8]) -> Option<Entete> {
        if o.len() < TAILLE_ENTETE { return None; }
        Some(Entete {
            magic: u32::from_le_bytes([o[0], o[1], o[2], o[3]]),
            version: u16::from_le_bytes([o[4], o[5]]),
            genre: u16::from_le_bytes([o[6], o[7]]),
            taille_charge: u32::from_le_bytes([o[8], o[9], o[10], o[11]]),
            serie: u32::from_le_bytes([o[12], o[13], o[14], o[15]]),
        })
    }
}

/// Ce qu'un decodeur conclut d'un tampon de reception.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lecture {
    Incomplet,
    Message { genre: Genre, debut: usize, taille: usize, total: usize },
    Invalide,
}

pub fn examine(tampon: &[u8]) -> Lecture {
    if tampon.len() < TAILLE_ENTETE { return Lecture::Incomplet; }
    let Some(entete) = Entete::decode(tampon) else { return Lecture::Incomplet };
    if !entete.valide() || entete.taille_charge > CHARGE_MAX { return Lecture::Invalide; }
    let Some(genre) = Genre::depuis_u16(entete.genre) else { return Lecture::Invalide };
    let taille = entete.taille_charge as usize;
    let total = TAILLE_ENTETE + taille;
    if tampon.len() < total { return Lecture::Incomplet; }
    Lecture::Message { genre, debut: TAILLE_ENTETE, taille, total }
}

pub fn message(genre: Genre, serie: u32, charge: &[u8]) -> Vec<u8> {
    let mut octets = Vec::with_capacity(TAILLE_ENTETE + charge.len());
    octets.extend_from_slice(&Entete::neuf(genre, charge.len() as u32, serie).encode());
    octets.extend_from_slice(charge);
    octets
}

// --- Charges utiles ----------------------------------------------------------

/// `SurfaceAccordee` : ce que le compositeur donne au client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurfaceAccordee {
    pub surface: u32,
    /// Largeur PHYSIQUE, en pixels.
    pub largeur: u32,
    pub hauteur: u32,
    pub pas: u32,
    /// Echelle en cent-vingtiemes (voir `gui::protocole::ECHELLE_UNITE`).
    pub echelle: u32,
    /// Nombre de tampons dans la region partagee.
    pub tampons: u32,
    /// Decalage du tampon 0 dans la region.
    pub decalage: u32,
    /// Indice du tampon que le client possede a la creation.
    pub tampon_initial: u32,
}

pub const TAILLE_SURFACE_ACCORDEE: usize = 32;

impl SurfaceAccordee {
    pub fn encode(&self) -> [u8; TAILLE_SURFACE_ACCORDEE] {
        let mut o = [0u8; TAILLE_SURFACE_ACCORDEE];
        o[0..4].copy_from_slice(&self.surface.to_le_bytes());
        o[4..8].copy_from_slice(&self.largeur.to_le_bytes());
        o[8..12].copy_from_slice(&self.hauteur.to_le_bytes());
        o[12..16].copy_from_slice(&self.pas.to_le_bytes());
        o[16..20].copy_from_slice(&self.echelle.to_le_bytes());
        o[20..24].copy_from_slice(&self.tampons.to_le_bytes());
        o[24..28].copy_from_slice(&self.decalage.to_le_bytes());
        o[28..32].copy_from_slice(&self.tampon_initial.to_le_bytes());
        o
    }
    pub fn decode(o: &[u8]) -> Option<SurfaceAccordee> {
        if o.len() < TAILLE_SURFACE_ACCORDEE { return None; }
        Some(SurfaceAccordee {
            surface: lit_u32(o, 0),
            largeur: lit_u32(o, 4),
            hauteur: lit_u32(o, 8),
            pas: lit_u32(o, 12),
            echelle: lit_u32(o, 16),
            tampons: lit_u32(o, 20),
            decalage: lit_u32(o, 24),
            tampon_initial: lit_u32(o, 28),
        })
    }

    /// Taille d'un tampon, en octets.
    pub const fn octets_tampon(&self) -> u64 {
        self.pas as u64 * self.hauteur as u64
    }
}

/// `TrameLivree` : le client a fini d'ecrire.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TrameLivree {
    pub surface: u32,
    pub tampon: u32,
    /// Numero de trame du client. Monotone ; il permet a un compositeur de
    /// detecter qu'il a saute une trame plutot que de le deviner.
    pub trame: u32,
    pub degat: Rect,
}

pub const TAILLE_TRAME_LIVREE: usize = 28;

impl TrameLivree {
    pub fn encode(&self) -> [u8; TAILLE_TRAME_LIVREE] {
        let mut o = [0u8; TAILLE_TRAME_LIVREE];
        o[0..4].copy_from_slice(&self.surface.to_le_bytes());
        o[4..8].copy_from_slice(&self.tampon.to_le_bytes());
        o[8..12].copy_from_slice(&self.trame.to_le_bytes());
        o[12..28].copy_from_slice(&self.degat.encode());
        o
    }
    pub fn decode(o: &[u8]) -> Option<TrameLivree> {
        if o.len() < TAILLE_TRAME_LIVREE { return None; }
        Some(TrameLivree {
            surface: lit_u32(o, 0),
            tampon: lit_u32(o, 4),
            trame: lit_u32(o, 8),
            degat: Rect::decode(&o[12..28])?,
        })
    }
}

/// `TamponRendu` : ce tampon n'est plus affiche.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TamponRendu {
    pub surface: u32,
    pub tampon: u32,
    /// Trame apres laquelle il a ete rendu.
    pub trame: u32,
    pub reserve: u32,
}

pub const TAILLE_TAMPON_RENDU: usize = 16;

impl TamponRendu {
    pub fn encode(&self) -> [u8; TAILLE_TAMPON_RENDU] {
        let mut o = [0u8; TAILLE_TAMPON_RENDU];
        o[0..4].copy_from_slice(&self.surface.to_le_bytes());
        o[4..8].copy_from_slice(&self.tampon.to_le_bytes());
        o[8..12].copy_from_slice(&self.trame.to_le_bytes());
        o[12..16].copy_from_slice(&self.reserve.to_le_bytes());
        o
    }
    pub fn decode(o: &[u8]) -> Option<TamponRendu> {
        if o.len() < TAILLE_TAMPON_RENDU { return None; }
        Some(TamponRendu {
            surface: lit_u32(o, 0),
            tampon: lit_u32(o, 4),
            trame: lit_u32(o, 8),
            reserve: lit_u32(o, 12),
        })
    }
}

fn lit_u32(o: &[u8], d: usize) -> u32 {
    u32::from_le_bytes([o[d], o[d + 1], o[d + 2], o[d + 3]])
}

// --- Contrat de taille -------------------------------------------------------

const _: () = assert!(TAILLE_ENTETE == 16);
const _: () = assert!(TAMPONS == 2);
const _: () = assert!(CHARGE_MAX as usize >= TAILLE_SURFACE_ACCORDEE);
const _: () = assert!(MAGIC != super::protocole::MAGIC,
    "les deux protocoles doivent se distinguer a l'octet pres");
