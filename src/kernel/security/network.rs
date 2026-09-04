use super::capability::Capabilities;
use super::policy::Snapshot;

const AF_PACKET: u32 = 17;
const SOCK_TYPE_MASK: u32 = 0xf;
const SOCK_RAW: u32 = 3;

/// Ce processus peut-il ouvrir cette socket ?
///
/// BOUCHAUD_C6_RESEAU_AU_SEUL_PROPRIETAIRE_V1
///
/// La regle etait : les sockets BRUTES demandent `NETWORK_ADMIN`, toutes les
/// autres sont ouvertes a tout le monde. Un moteur de rendu compromis pouvait
/// donc ouvrir une connexion TCP vers n'importe quel hote -- ce qui vide de son
/// sens l'architecture ou le reseau appartient a RequestServer, et transforme
/// une faille d'analyse HTML en canal de sortie.
///
/// Il y a maintenant DEUX conditions, et elles ne se remplacent pas :
///
///   * ouvrir une socket, quelle qu'elle soit, demande `NET_CONNECT`. Le
///     systeme, l'utilisateur, le courtier et RequestServer l'ont ; les roles
///     de RENDU ne l'ont pas ;
///   * une socket BRUTE demande en plus `NETWORK_ADMIN`, comme avant.
pub fn socket_allowed(security: Snapshot, domain: u32, socket_type: u32) -> bool {
    if !security.capabilities.contains(Capabilities::NET_CONNECT) {
        return false;
    }
    let raw = socket_type & SOCK_TYPE_MASK == SOCK_RAW || domain == AF_PACKET;
    !raw || security.capabilities.contains(Capabilities::NETWORK_ADMIN)
}
