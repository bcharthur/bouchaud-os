// Metadonnees d'une entree de table (BOUCHAUD_PERSIST_META_V1).
//
// # Le manque qu'elles comblent
//
// La table ne retenait qu'un chemin et une taille : seuls les fichiers NON
// VIDES survivaient au redemarrage. Un dossier vide (« Documents » a peine
// cree), un fichier vide, les droits et le proprietaire de tout ce qui etait
// sous `/persist` disparaissaient. Un dossier personnel ne pouvait pas etre
// persistant.
//
// # Ou elles vivent, et pourquoi c'est compatible dans les deux sens
//
// Une entree fait 256 octets : chemin (0..240), taille (240..248), et HUIT
// OCTETS jamais ecrits -- la table est mise a zero avant d'etre remplie. Les
// metadonnees y tiennent exactement :
//
//   248  genre : 0 = fichier au format d'avant, 'F' = fichier, 'D' = dossier
//   249  version des metadonnees (1)
//   250  mode (u16, little endian)
//   252  uid  (u16)
//   254  gid  (u16)
//
// * Un disque ECRIT PAR UN NOYAU D'AVANT porte 0 en 248 : lu comme un fichier,
//   avec les droits par defaut, exactement comme avant.
// * Un disque ECRIT PAR CE NOYAU et relu par un noyau d'avant (retour A/B sur
//   l'autre emplacement) : l'ancien montage ignore ces octets et saute toute
//   entree de taille nulle -- dossiers et fichiers vides --, il restaure les
//   memes fichiers non vides qu'avant.
// * La somme de controle du superbloc couvre la table ENTIERE : une
//   metadonnee dechiree rejette la table, comme un chemin dechire.
//
// Ce fragment est PUR : `tools/fs/test_persist_meta.rs` l'inclut tel quel.

/// Fichier au format d'avant les metadonnees (octet 248 nul).
pub const GENRE_HERITE: u8 = 0;
/// Fichier avec metadonnees.
pub const GENRE_FICHIER: u8 = b'F';
/// Dossier avec metadonnees.
pub const GENRE_DOSSIER: u8 = b'D';
/// Version de l'encodage.
pub const VERSION_META: u8 = 1;
/// Position des metadonnees dans une entree de 256 octets.
pub const DEBUT_META: usize = 248;

/// Ce que la table retient d'un noeud, en plus de son chemin et de sa taille.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Meta {
    pub dossier: bool,
    pub mode: u16,
    pub uid: u16,
    pub gid: u16,
}

impl Meta {
    /// Les droits qu'un fichier au format d'avant recoit au montage : ceux que
    /// `touch_at` donne a un fichier neuf.
    pub const HERITE: Meta = Meta { dossier: false, mode: 0o644, uid: 0, gid: 0 };

    /// Ecrit les metadonnees dans les 8 octets `cible`.
    pub fn encode(&self, cible: &mut [u8]) {
        cible[0] = if self.dossier { GENRE_DOSSIER } else { GENRE_FICHIER };
        cible[1] = VERSION_META;
        cible[2..4].copy_from_slice(&self.mode.to_le_bytes());
        cible[4..6].copy_from_slice(&self.uid.to_le_bytes());
        cible[6..8].copy_from_slice(&self.gid.to_le_bytes());
    }

    /// Lit les 8 octets `brut`. Un genre inconnu, ou une version inconnue,
    /// rend `None` : l'entree est alors traitee comme au format d'avant (un
    /// fichier aux droits par defaut), ce qui ne perd jamais un contenu.
    pub fn decode(brut: &[u8]) -> Option<Meta> {
        if brut.len() < 8 || brut[1] != VERSION_META {
            return None;
        }
        let dossier = match brut[0] {
            GENRE_FICHIER => false,
            GENRE_DOSSIER => true,
            _ => return None,
        };
        Some(Meta {
            dossier,
            mode: u16::from_le_bytes([brut[2], brut[3]]) & 0o7777,
            uid: u16::from_le_bytes([brut[4], brut[5]]),
            gid: u16::from_le_bytes([brut[6], brut[7]]),
        })
    }
}

/// Une entree de la table doit-elle etre restauree au montage ? Un fichier
/// au format d'avant de taille nulle n'a jamais ete ecrit (la collecte les
/// sautait) : c'est une entree vide. Avec metadonnees, un dossier ou un
/// fichier vide sont restaures.
pub fn a_restaurer(chemin: &str, taille: usize, meta: Option<Meta>) -> bool {
    !chemin.is_empty() && (taille > 0 || meta.is_some())
}
