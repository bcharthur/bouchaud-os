//! Couche UDP (datagrammes). Checksum mis a 0 (autorise en IPv4) pour rester
//! simple : SLIRP/QEMU l'accepte.

/// Construit un datagramme UDP (en-tete 8 octets + charge utile).
pub fn build(buf: &mut [u8], src_port: u16, dst_port: u16, payload: &[u8]) -> Option<usize> {
    let total = 8 + payload.len();
    if buf.len() < total { return None; }
    buf[0] = (src_port >> 8) as u8;
    buf[1] = src_port as u8;
    buf[2] = (dst_port >> 8) as u8;
    buf[3] = dst_port as u8;
    buf[4] = (total >> 8) as u8;
    buf[5] = total as u8;
    buf[6] = 0; // checksum desactive
    buf[7] = 0;
    buf[8..total].copy_from_slice(payload);
    Some(total)
}

/// La somme de controle UDP : (juste, absente).
///
/// # Pourquoi deux booleens et non un
///
/// En IPv4, la somme UDP est FACULTATIVE : un emetteur qui ne la calcule pas
/// ecrit zero, et le datagramme reste parfaitement valide. Confondre « pas de
/// somme » avec « somme fausse » ferait accuser le reseau la ou il n'y a rien
/// a reprocher.
///
/// La somme couvre un pseudo-en-tete -- adresses, protocole, longueur -- puis
/// le datagramme entier. Recalculee en incluant le champ lui-meme, elle vaut
/// zero quand elle est juste.
pub fn somme_verdict(src: &[u8; 4], dst: &[u8; 4], datagramme: &[u8]) -> (bool, bool) {
    if datagramme.len() < 8 {
        return (false, false);
    }
    let portee = ((datagramme[6] as u16) << 8) | datagramme[7] as u16;
    if portee == 0 {
        // Zero : l'emetteur n'en a pas calcule. C'est legal.
        return (true, true);
    }
    let longueur = ((datagramme[4] as usize) << 8) | datagramme[5] as usize;
    if longueur < 8 || longueur > datagramme.len() {
        return (false, false);
    }
    let mut somme: u32 = 0;
    // Le pseudo-en-tete.
    somme += ((src[0] as u32) << 8) | src[1] as u32;
    somme += ((src[2] as u32) << 8) | src[3] as u32;
    somme += ((dst[0] as u32) << 8) | dst[1] as u32;
    somme += ((dst[2] as u32) << 8) | dst[3] as u32;
    somme += 17; // PROTO_UDP
    somme += longueur as u32;
    // Le datagramme, par mots de seize bits.
    let utile = &datagramme[..longueur];
    let mut place = 0usize;
    while place + 1 < utile.len() {
        somme += ((utile[place] as u32) << 8) | utile[place + 1] as u32;
        place += 2;
    }
    if place < utile.len() {
        somme += (utile[place] as u32) << 8;
    }
    while somme >> 16 != 0 {
        somme = (somme & 0xFFFF) + (somme >> 16);
    }
    (somme as u16 == 0xFFFF, false)
}

/// En-tete UDP decode.
pub struct Header {
    pub src_port: u16,
    pub dst_port: u16,
    pub payload_off: usize,
    pub payload_len: usize,
}

/// Decode un datagramme UDP.
pub fn parse(buf: &[u8]) -> Option<Header> {
    if buf.len() < 8 { return None; }
    let len = ((buf[4] as usize) << 8) | buf[5] as usize;
    if len < 8 || len > buf.len() { return None; }
    Some(Header {
        src_port: ((buf[0] as u16) << 8) | buf[1] as u16,
        dst_port: ((buf[2] as u16) << 8) | buf[3] as u16,
        payload_off: 8,
        payload_len: len - 8,
    })
}
