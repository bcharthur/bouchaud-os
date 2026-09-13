//! Analyse des options d'une reponse DHCP.
//!
//! # Pourquoi cette moitie vit a part
//!
//! Elle prend des octets venus du reseau et rend des entiers. Aucun registre,
//! aucune carte, aucun etat global : c'est donc la seule moitie du client DHCP
//! qu'une suite hote peut mettre a l'epreuve -- et c'est celle ou une faute
//! serait la plus discrete. Une longueur mal bornee sur un paquet hostile lit
//! au-dela du tampon ; un masque accepte sans verification donne un nom de
//! reseau faux ; un octet de controle laisse passer dans le nom de domaine
//! arrive jusqu'a la barre des taches.

/// Une adresse IPv4, sans dependre du reste de la pile.
pub type Adresse = [u8; 4];

/// Un nom de domaine DHCP plus long que cela n'est pas un nom de reseau.
pub const LONGUEUR_DOMAINE: usize = 63;

const COOKIE: [u8; 4] = [99, 130, 83, 99];

#[derive(Clone, Copy)]
pub struct Lease {
    pub msg_type: u8,
    pub your_ip: Adresse,
    pub server_id: Adresse,
    pub router: Adresse,
    pub dns: Adresse,
    /// Masque de sous-reseau (option 1), a defaut de nom.
    pub masque: Adresse,
    /// Nom de domaine annonce par le serveur (option 15).
    ///
    /// Il etait DEJA demande dans la liste des parametres souhaites, et la
    /// reponse le portait donc souvent -- personne ne le lisait. C'est le seul
    /// nom qu'un reseau filaire se donne : un cable n'a pas de SSID.
    pub domaine: [u8; LONGUEUR_DOMAINE],
    pub domaine_len: usize,
}

// `derive(Default)` ne sait pas construire un tableau de plus de trente-deux
// octets ; l'ecrire a la main coute trois lignes et evite de rogner le nom.
impl Default for Lease {
    fn default() -> Self {
        Self {
            msg_type: 0,
            your_ip: [0; 4],
            server_id: [0; 4],
            router: [0; 4],
            dns: [0; 4],
            masque: [0; 4],
            domaine: [0; LONGUEUR_DOMAINE],
            domaine_len: 0,
        }
    }
}

/// Construit un message DHCP (BOOTREQUEST). Renvoie la longueur (>= 300).
pub fn build_msg(buf: &mut [u8], xid: u32, mac: [u8; 6], msg_type: u8,
             req_ip: Option<Adresse>, server_id: Option<Adresse>) -> usize {
    for b in buf[..300].iter_mut() { *b = 0; }
    buf[0] = 1;            // op = BOOTREQUEST
    buf[1] = 1;            // htype = Ethernet
    buf[2] = 6;            // hlen
    buf[4..8].copy_from_slice(&xid.to_be_bytes());
    buf[10] = 0x80;        // flags : broadcast (on n'a pas encore d'IP)
    buf[28..34].copy_from_slice(&mac);
    buf[236..240].copy_from_slice(&COOKIE);

    let mut p = 240;
    buf[p] = 53; buf[p + 1] = 1; buf[p + 2] = msg_type; p += 3;
    if let Some(ip) = req_ip {
        buf[p] = 50; buf[p + 1] = 4; buf[p + 2..p + 6].copy_from_slice(&ip); p += 6;
    }
    if let Some(sid) = server_id {
        buf[p] = 54; buf[p + 1] = 4; buf[p + 2..p + 6].copy_from_slice(&sid); p += 6;
    }
    buf[p] = 55; buf[p + 1] = 4; buf[p + 2] = 1; buf[p + 3] = 3; buf[p + 4] = 6; buf[p + 5] = 15; p += 6;
    buf[p] = 255; p += 1; // fin des options
    if p < 300 { 300 } else { p }
}

/// Analyse une reponse DHCP (yiaddr + options utiles).
pub fn parse_reply(buf: &[u8]) -> Option<Lease> {
    if buf.len() < 240 || buf[236..240] != COOKIE { return None; }
    let mut lease = Lease::default();
    lease.your_ip = [buf[16], buf[17], buf[18], buf[19]];
    let mut p = 240;
    while p + 1 < buf.len() {
        let code = buf[p];
        if code == 255 { break; }
        if code == 0 { p += 1; continue; }
        let len = buf[p + 1] as usize;
        let data = p + 2;
        if data + len > buf.len() { break; }
        match code {
            53 if len >= 1 => lease.msg_type = buf[data],
            54 if len >= 4 => lease.server_id = [buf[data], buf[data + 1], buf[data + 2], buf[data + 3]],
            3 if len >= 4 => lease.router = [buf[data], buf[data + 1], buf[data + 2], buf[data + 3]],
            6 if len >= 4 => lease.dns = [buf[data], buf[data + 1], buf[data + 2], buf[data + 3]],
            1 if len >= 4 => lease.masque = [buf[data], buf[data + 1], buf[data + 2], buf[data + 3]],
            // Option 15 : le nom du domaine. Un octet non imprimable ferait
            // du nom de reseau un vecteur d'affichage ; il est filtre ici,
            // et non a la peinture.
            15 if len >= 1 => {
                let n = len.min(LONGUEUR_DOMAINE);
                let mut garde = 0usize;
                for octet in buf[data..data + n].iter() {
                    if octet.is_ascii_graphic() || *octet == b' ' {
                        lease.domaine[garde] = *octet;
                        garde += 1;
                    }
                }
                lease.domaine_len = garde;
            }
            _ => {}
        }
        p = data + len;
    }
    Some(lease)
}

/// Longueur du prefixe d'un masque de sous-reseau.
///
/// Rend `None` sur un masque absent ou MAL FORME. Un masque valide est une
/// suite de uns suivie d'une suite de zeros ; `255.0.255.0` n'en est pas un,
/// et en tirer « /16 » nommerait le reseau de travers.
pub fn longueur_prefixe(masque: Adresse) -> Option<u32> {
    let brut = u32::from_be_bytes(masque);
    if brut == 0 {
        return None;
    }
    let uns = brut.leading_ones();
    if uns < 32 && brut.wrapping_shl(uns) != 0 {
        return None;
    }
    Some(uns)
}

/// L'adresse du reseau : l'adresse de l'hote, masquee.
pub fn adresse_reseau(ip: Adresse, masque: Adresse) -> Adresse {
    [
        ip[0] & masque[0],
        ip[1] & masque[1],
        ip[2] & masque[2],
        ip[3] & masque[3],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entete(msg_type: u8) -> Vec<u8> {
        let mut buf = vec![0u8; 240];
        buf[16..20].copy_from_slice(&[192, 168, 1, 42]);
        buf[236..240].copy_from_slice(&COOKIE);
        buf.push(53);
        buf.push(1);
        buf.push(msg_type);
        buf
    }

    #[test]
    fn le_domaine_est_lu() {
        let mut buf = entete(5);
        buf.extend_from_slice(&[15, 9]);
        buf.extend_from_slice(b"fritz.box");
        buf.push(255);
        let bail = parse_reply(&buf).unwrap();
        assert_eq!(&bail.domaine[..bail.domaine_len], b"fritz.box");
    }

    #[test]
    fn un_octet_de_controle_est_filtre() {
        let mut buf = entete(5);
        buf.extend_from_slice(&[15, 5]);
        buf.extend_from_slice(&[b'h', 0x1b, b'o', 0x07, b'm']);
        buf.push(255);
        let bail = parse_reply(&buf).unwrap();
        assert_eq!(
            &bail.domaine[..bail.domaine_len],
            b"hom",
            "un nom de reseau ne doit pas pouvoir porter de sequence d'echappement"
        );
    }

    #[test]
    fn un_domaine_trop_long_est_tronque_sans_deborder() {
        let mut buf = entete(5);
        let long = vec![b'a'; 200];
        buf.push(15);
        buf.push(200);
        buf.extend_from_slice(&long);
        buf.push(255);
        let bail = parse_reply(&buf).unwrap();
        assert_eq!(bail.domaine_len, LONGUEUR_DOMAINE);
    }

    #[test]
    fn une_longueur_qui_depasse_le_tampon_arrete_l_analyse() {
        let mut buf = entete(5);
        buf.extend_from_slice(&[15, 200]);
        buf.extend_from_slice(b"court");
        let bail = parse_reply(&buf).unwrap();
        assert_eq!(bail.domaine_len, 0, "rien ne doit etre lu au-dela du tampon");
    }

    #[test]
    fn le_masque_et_le_routeur_sont_lus() {
        let mut buf = entete(5);
        buf.extend_from_slice(&[1, 4, 255, 255, 255, 0]);
        buf.extend_from_slice(&[3, 4, 192, 168, 1, 1]);
        buf.extend_from_slice(&[6, 4, 1, 1, 1, 1]);
        buf.push(255);
        let bail = parse_reply(&buf).unwrap();
        assert_eq!(bail.masque, [255, 255, 255, 0]);
        assert_eq!(bail.router, [192, 168, 1, 1]);
        assert_eq!(bail.dns, [1, 1, 1, 1]);
        assert_eq!(bail.your_ip, [192, 168, 1, 42]);
    }

    #[test]
    fn sans_cookie_rien_n_est_analyse() {
        let mut buf = entete(5);
        buf[236] = 0;
        assert!(parse_reply(&buf).is_none());
    }

    #[test]
    fn un_paquet_tronque_est_refuse() {
        assert!(parse_reply(&[0u8; 100]).is_none());
    }

    #[test]
    fn l_option_de_remplissage_ne_decale_rien() {
        let mut buf = entete(5);
        buf.extend_from_slice(&[0, 0, 0]);
        buf.extend_from_slice(&[1, 4, 255, 255, 0, 0]);
        buf.push(255);
        let bail = parse_reply(&buf).unwrap();
        assert_eq!(bail.masque, [255, 255, 0, 0]);
    }

    #[test]
    fn les_prefixes_usuels() {
        assert_eq!(longueur_prefixe([255, 255, 255, 0]), Some(24));
        assert_eq!(longueur_prefixe([255, 255, 0, 0]), Some(16));
        assert_eq!(longueur_prefixe([255, 255, 255, 255]), Some(32));
        assert_eq!(longueur_prefixe([255, 255, 254, 0]), Some(23));
    }

    #[test]
    fn un_masque_absent_ou_mal_forme_ne_nomme_rien() {
        assert_eq!(longueur_prefixe([0, 0, 0, 0]), None);
        assert_eq!(
            longueur_prefixe([255, 0, 255, 0]),
            None,
            "un masque a trous ne se resume pas a une longueur de prefixe"
        );
        assert_eq!(longueur_prefixe([0, 255, 255, 0]), None);
    }

    #[test]
    fn l_adresse_de_reseau_est_l_adresse_masquee() {
        assert_eq!(
            adresse_reseau([192, 168, 1, 42], [255, 255, 255, 0]),
            [192, 168, 1, 0]
        );
        assert_eq!(
            adresse_reseau([10, 12, 200, 7], [255, 255, 0, 0]),
            [10, 12, 0, 0]
        );
    }
}
