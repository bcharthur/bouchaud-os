//! L'adresse du canal de diagnostic, derivee de la MAC et de rien d'autre.
//!
//! # Pourquoi le debugger ne peut pas dependre de DHCP
//!
//! Le releve TRIGKEY dit exactement pourquoi :
//!
//! ```text
//! rx_packets=64  rx_cur=0  desc_nic=64  desc_cpu=0   (pendant 151 s)
//! ```
//!
//! La reception meurt apres soixante-quatre trames. Un OFFER DHCP est une
//! trame entrante ; un ACK aussi. Faire dependre le canal d'enquete d'une
//! negociation qui a besoin de la chose en panne, c'est garantir qu'il sera
//! absent le jour ou il sert.
//!
//! D'ou une adresse link-local, RFC 3927, calculee a partir de la MAC. Elle
//! ne demande rien a personne, ne change pas d'une session a l'autre, et est
//! connue du PC de developpement avant meme que la machine demarre.
//!
//! # Ce que cette adresse n'est PAS
//!
//! Pas de passerelle, pas de DNS, pas de route par defaut. Ce n'est pas une
//! configuration reseau de repli : c'est un canal de diagnostic sur le
//! segment local, et rien de plus. Lui donner une passerelle fictive ferait
//! sortir du trafic vers une adresse qui n'existe pas, et masquerait la vraie
//! panne derriere des delais d'attente.

/// Le prefixe RFC 3927.
pub const PREFIXE: [u8; 2] = [169, 254];
/// Longueur du prefixe, en bits. Un /16, comme la RFC l'exige.
pub const PREFIXE_BITS: u8 = 16;

/// Premier troisieme octet utilisable. `169.254.0.0/24` est reserve.
const TROISIEME_MIN: u8 = 1;
/// Dernier troisieme octet utilisable. `169.254.255.0/24` est reserve.
const TROISIEME_MAX: u8 = 254;
/// Adresses utilisables : `254 * 256`.
const CARDINAL: u32 = (TROISIEME_MAX - TROISIEME_MIN + 1) as u32 * 256;

/// L'adresse de diagnostic de cette machine.
///
/// # Pourquoi FNV et pas « les deux derniers octets de la MAC »
///
/// Prendre les deux derniers octets tels quels donne `169.254.0.x` a toute
/// une famille de cartes -- et `169.254.0.0/24` est justement le sous-reseau
/// que la RFC interdit. Un melange complet des six octets repartit les
/// adresses sur toute la plage utilisable et fait dependre le resultat de
/// l'OUI du constructeur autant que du numero de serie.
///
/// FNV-1a : quelques instructions, pas de table, deterministe. Ce n'est pas
/// une fonction de hachage cryptographique et n'a pas a l'etre -- personne ne
/// choisit sa MAC pour entrer en collision sur un segment local.
pub fn depuis_mac(mac: [u8; 6]) -> [u8; 4] {
    let mut h: u32 = 2_166_136_261;
    for octet in mac {
        h ^= octet as u32;
        h = h.wrapping_mul(16_777_619);
    }
    let n = h % CARDINAL;
    [
        PREFIXE[0],
        PREFIXE[1],
        TROISIEME_MIN + (n / 256) as u8,
        (n % 256) as u8,
    ]
}

/// Cette adresse est-elle dans `169.254.0.0/16` ?
pub fn est_link_local(ip: [u8; 4]) -> bool {
    ip[0] == PREFIXE[0] && ip[1] == PREFIXE[1]
}

/// Cette adresse est-elle UTILISABLE au sens de la RFC 3927 ?
///
/// Le premier et le dernier /24 sont reserves. Une adresse qui y tomberait
/// serait ignoree par les piles qui respectent la RFC -- donc silencieusement
/// inutilisable, ce qui est le pire des echecs pour un canal d'enquete.
pub fn est_utilisable(ip: [u8; 4]) -> bool {
    est_link_local(ip) && ip[2] >= TROISIEME_MIN && ip[2] <= TROISIEME_MAX
}

/// Le masque du prefixe, sous forme d'adresse.
pub const fn masque() -> [u8; 4] {
    [255, 255, 0, 0]
}

/// Deux adresses sont-elles sur le meme segment link-local ?
pub fn meme_segment(a: [u8; 4], b: [u8; 4]) -> bool {
    est_link_local(a) && est_link_local(b)
}
