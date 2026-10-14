//! Qui peut encore toucher une memoire DMA confiee a virtio-gpu.
//!
//! BOUCHAUD_VIRTIO_GPU_DMA_ETAT_V1
//!
//! # Le defaut
//!
//! `active_scanout` rendait sa memoire si elle n'avait « jamais ete
//! attachee » -- un booleen pose seulement quand `ATTACH_BACKING` repondait
//! OK. Une commande SANS REPONSE (attente bornee echue) laissait ce booleen
//! faux alors que le peripherique avait peut-etre recu la requete et retenu
//! l'adresse : la memoire etait rendue a l'allocateur, puis reutilisee, et le
//! peripherique pouvait encore y lire ou y ecrire.
//!
//! # Le modele
//!
//! Chaque tampon DMA a un etat :
//!
//!   * `NonAttachee` : le peripherique n'en a aucune reference. Seul etat ou
//!     la memoire peut etre rendue ;
//!   * `Attachee` : `ATTACH_BACKING` accepte, pas encore detache ;
//!   * `Inconnue` : une commande qui la concerne est restee sans reponse. On
//!     ne sait pas si le peripherique la tient : elle n'est JAMAIS rendue (une
//!     fuite bornee vaut mieux qu'une corruption). L'etat est absorbant.
//!
//! L'issue d'une commande est l'une de :
//!
//!   * `Acceptee` (reponse OK) ; `Refusee` (le peripherique a REPONDU par une
//!     erreur : il n'a rien retenu) ;
//!   * `SansReponse` : la requete est partie, la reponse n'est pas venue --
//!     ou n'etait pas celle de la commande (`REPONSE_INVALIDE`) ;
//!   * `NonEnvoyee` : la requete n'est jamais partie (file deja cassee,
//!     requete trop longue) -- le peripherique ne l'a pas vue.
//!
//! Module pur : `tools/display/test_virtio_gpu_dma.rs` le teste sur l'hote,
//! dont le scenario du defaut.

/// `VIRTIO_GPU_RESP_OK_NODATA`.
pub const REPONSE_OK: u32 = 0x1100;
/// Erreur rendue par la file quand l'attente bornee echoue : la requete est
/// partie, le peripherique l'a peut-etre traitee.
pub const SANS_REPONSE: &str = "commande-sans-reponse";
/// Erreur rendue par la file quand la reponse n'est pas celle de la commande
/// en vol (BOUCHAUD_VIRTIO_GPU_BARRIERE_V1 : entree utilisee inattendue,
/// barriere absente ou differente). La requete est partie, et rien ne dit ce
/// que le peripherique en a fait : meme issue qu'une commande sans reponse.
pub const REPONSE_INVALIDE: &str = "reponse-invalide";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Memoire {
    NonAttachee,
    Attachee,
    Inconnue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Issue {
    Acceptee,
    Refusee,
    SansReponse,
    NonEnvoyee,
}

/// L'issue d'une commande, d'apres ce que la file a rendu.
pub fn issue(rendu: Result<u32, &str>) -> Issue {
    match rendu {
        Ok(REPONSE_OK) => Issue::Acceptee,
        Ok(_) => Issue::Refusee,
        Err(e) if e == SANS_REPONSE || e == REPONSE_INVALIDE => Issue::SansReponse,
        Err(_) => Issue::NonEnvoyee,
    }
}

/// Apres un `RESOURCE_ATTACH_BACKING` portant ce tampon.
pub fn apres_attache(avant: Memoire, i: Issue) -> Memoire {
    match (avant, i) {
        (Memoire::Inconnue, _) => Memoire::Inconnue,
        (_, Issue::SansReponse) => Memoire::Inconnue,
        (_, Issue::Acceptee) => Memoire::Attachee,
        (m, Issue::Refusee) | (m, Issue::NonEnvoyee) => m,
    }
}

/// Apres un `RESOURCE_DETACH_BACKING` de la ressource qui porte ce tampon.
///
/// Un refus laisse l'etat tel quel : on ne sait pas pourquoi le peripherique
/// refuse, on ne conclut donc pas qu'il a lache la memoire.
pub fn apres_detache(avant: Memoire, i: Issue) -> Memoire {
    match (avant, i) {
        (Memoire::Inconnue, _) => Memoire::Inconnue,
        (_, Issue::SansReponse) => Memoire::Inconnue,
        (_, Issue::Acceptee) => Memoire::NonAttachee,
        (m, Issue::Refusee) | (m, Issue::NonEnvoyee) => m,
    }
}

/// Une autre commande sur la ressource (creation, scanout, transfert, flush,
/// liberation). Elle ne change pas la reference du peripherique sur la
/// memoire, SAUF si elle reste sans reponse alors que la memoire est
/// attachee : un transfert sans reponse peut encore la lire.
pub fn apres_autre(avant: Memoire, i: Issue) -> Memoire {
    match (avant, i) {
        (Memoire::Attachee, Issue::SansReponse) => Memoire::Inconnue,
        (m, _) => m,
    }
}

/// La memoire peut-elle etre rendue a l'allocateur ?
pub fn peut_rendre(m: Memoire) -> bool {
    m == Memoire::NonAttachee
}
