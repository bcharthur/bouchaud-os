//! Ce que la file de controle virtio-gpu peut croire d'une reponse.
//!
//! BOUCHAUD_VIRTIO_GPU_BARRIERE_V1
//!
//! # Ce que la specification garantit, et ce qu'elle ne garantit pas
//!
//! `virtio-spec`, `device-types/gpu/description.tex` :
//!
//! > The device MAY process controlq commands asynchronously and return them
//! > to the driver before the processing is complete. If the driver needs to
//! > know when the processing is finished it can set the
//! > VIRTIO_GPU_FLAG_FENCE flag in the request. The device MUST finish the
//! > processing before returning the command then.
//!
//! > If the driver sets the VIRTIO_GPU_FLAG_FENCE bit in the request flags
//! > field the device MUST: set VIRTIO_GPU_FLAG_FENCE bit in the response,
//! > copy the content of the fence_id field from the request to the
//! > response, and send the response only after command processing is
//! > complete.
//!
//! Rien n'y dit que les commandes de la file de controle sont TRAITEES dans
//! l'ordre. `VIRTIO_F_IN_ORDER` (anneau scinde) ne porte que sur l'ordre
//! d'UTILISATION des tampons, et QEMU ne l'offre pas
//! (`fonctionnalites_proposees=0x0000010130000002`).
//!
//! # Ce que le pilote en tire
//!
//!   * TOUTE commande part avec `VIRTIO_GPU_FLAG_FENCE` et un `fence_id`
//!     croissant : sa reponse prouve que le traitement est fini. Sans cela,
//!     un `DETACH_BACKING` accepte ne prouverait pas que le peripherique a
//!     lache la memoire, et un `TRANSFER_TO_HOST_2D` rendu pourrait encore
//!     lire la memoire que la trame suivante reecrit ;
//!   * UNE seule commande en vol (chaine de tete 0) : le `RESOURCE_FLUSH`
//!     n'est envoye qu'apres la reponse du transfert. Le groupement des deux
//!     sous une sonnette (BOUCHAUD_VIRTIO_GPU_LOT_V1) est RETIRE : rien ne
//!     garantissait que le flush serait traite apres le transfert ;
//!   * chaque reponse est jugee ici : exactement UNE entree utilisee de plus,
//!     d'identifiant egal a la tete de chaine, de longueur au moins un
//!     en-tete et au plus la capacite du descripteur ecrivable (un
//!     peripherique conforme n'annonce pas plus qu'il ne pouvait ecrire),
//!     barriere renvoyee avec le meme `fence_id`. Autrement, la
//!     reponse n'est pas celle de la commande en vol (achevement desordonne,
//!     reponse tardive d'une commande precedente, peripherique non conforme)
//!     : la file est cassee et l'issue tenue pour INCONNUE -- la memoire
//!     concernee n'est pas rendue.
//!
//! Module pur : `tools/display/test_virtio_gpu_file.rs`.

/// `VIRTIO_GPU_FLAG_FENCE`.
pub const DRAPEAU_BARRIERE: u32 = 1;
/// `struct virtio_gpu_ctrl_hdr` : 24 octets.
pub const ENTETE: u32 = 24;
/// Decalages dans l'en-tete (requete et reponse).
pub const DECALAGE_DRAPEAUX: usize = 4;
pub const DECALAGE_BARRIERE: usize = 8;

/// Une entree de l'anneau utilise (`struct virtq_used_elem`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Utilise {
    pub id: u32,
    pub len: u32,
}

/// Ce que la reponse a de faux.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Violation {
    /// Plus d'une entree utilisee pour une seule commande en vol.
    TropDEntrees { avance: u16 },
    /// L'entree designe une autre chaine que la commande en vol.
    IdInattendu { attendu: u32, recu: u32 },
    /// Le peripherique a ecrit moins qu'un en-tete.
    ReponseCourte { len: u32 },
    /// Le peripherique annonce plus d'octets que le descripteur ecrivable
    /// n'en offrait.
    ReponseTropLongue { len: u32, capacite: u32 },
    /// Reponse sans `VIRTIO_GPU_FLAG_FENCE` : rien ne dit que le traitement
    /// est fini.
    BarriereAbsente,
    /// Reponse d'une autre commande (tardive, ou rejouee).
    BarriereDifferente { attendue: u64, recue: u64 },
}

impl Violation {
    pub const fn nom(&self) -> &'static str {
        match self {
            Violation::TropDEntrees { .. } => "trop-d-entrees",
            Violation::IdInattendu { .. } => "id-inattendu",
            Violation::ReponseCourte { .. } => "reponse-courte",
            Violation::ReponseTropLongue { .. } => "reponse-trop-longue",
            Violation::BarriereAbsente => "barriere-absente",
            Violation::BarriereDifferente { .. } => "barriere-differente",
        }
    }
}

/// Juge la reponse d'UNE commande en vol.
///
/// `avance` : `used.idx - dernier_util` (modulo 2^16) ; `tete` : la tete de
/// chaine de la commande ; `capacite` : la longueur du descripteur
/// ecrivable ; `drapeaux` et `barriere` : relus dans la reponse ;
/// `barriere_attendue` : le `fence_id` envoye.
pub fn juge(
    avance: u16,
    tete: u16,
    u: Utilise,
    capacite: u32,
    drapeaux: u32,
    barriere: u64,
    barriere_attendue: u64,
) -> Result<(), Violation> {
    if avance != 1 {
        return Err(Violation::TropDEntrees { avance });
    }
    if u.id != tete as u32 {
        return Err(Violation::IdInattendu { attendu: tete as u32, recu: u.id });
    }
    if u.len < ENTETE {
        return Err(Violation::ReponseCourte { len: u.len });
    }
    if u.len > capacite {
        return Err(Violation::ReponseTropLongue { len: u.len, capacite });
    }
    if drapeaux & DRAPEAU_BARRIERE == 0 {
        return Err(Violation::BarriereAbsente);
    }
    if barriere != barriere_attendue {
        return Err(Violation::BarriereDifferente { attendue: barriere_attendue, recue: barriere });
    }
    Ok(())
}

/// Pose le drapeau et le `fence_id` dans l'en-tete d'une requete.
pub fn pose_barriere(requete: &mut [u8], barriere: u64) {
    let d = u32::from_le_bytes([
        requete[DECALAGE_DRAPEAUX],
        requete[DECALAGE_DRAPEAUX + 1],
        requete[DECALAGE_DRAPEAUX + 2],
        requete[DECALAGE_DRAPEAUX + 3],
    ]) | DRAPEAU_BARRIERE;
    requete[DECALAGE_DRAPEAUX..DECALAGE_DRAPEAUX + 4].copy_from_slice(&d.to_le_bytes());
    requete[DECALAGE_BARRIERE..DECALAGE_BARRIERE + 8].copy_from_slice(&barriere.to_le_bytes());
}
