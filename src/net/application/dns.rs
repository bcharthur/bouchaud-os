//! Client DNS minimal (requete A, parsing de la premiere reponse A).

use crate::net::ipv4::Ipv4Addr;

/// Construit une requete DNS de type A pour `name`. Renvoie la longueur.
pub fn build_query(buf: &mut [u8], id: u16, name: &str) -> Option<usize> {
    if buf.len() < 12 { return None; }
    // En-tete : id, flags=0x0100 (recursion desiree), QDCOUNT=1, reste 0.
    buf[0] = (id >> 8) as u8;
    buf[1] = id as u8;
    buf[2] = 0x01;
    buf[3] = 0x00;
    buf[4] = 0x00; buf[5] = 0x01; // QDCOUNT = 1
    buf[6] = 0; buf[7] = 0;
    buf[8] = 0; buf[9] = 0;
    buf[10] = 0; buf[11] = 0;
    let mut pos = 12;
    // Question : suite de labels longueur+octets, terminee par 0.
    for label in name.split('.') {
        let l = label.len();
        if l == 0 || l > 63 || pos + 1 + l + 5 > buf.len() { return None; }
        buf[pos] = l as u8;
        pos += 1;
        buf[pos..pos + l].copy_from_slice(label.as_bytes());
        pos += l;
    }
    buf[pos] = 0; pos += 1; // fin du nom
    buf[pos] = 0; buf[pos + 1] = 1; // QTYPE = A
    buf[pos + 2] = 0; buf[pos + 3] = 1; // QCLASS = IN
    pos += 4;
    Some(pos)
}

/// Avance au-dela d'un nom DNS (gere la compression par pointeur 0xC0).
fn skip_name(buf: &[u8], mut pos: usize) -> Option<usize> {
    loop {
        if pos >= buf.len() { return None; }
        let len = buf[pos];
        if len == 0 { return Some(pos + 1); }
        if len & 0xC0 == 0xC0 {
            return Some(pos + 2); // pointeur de compression : 2 octets
        }
        pos += 1 + len as usize;
    }
}

/// Analyse une reponse DNS et renvoie la premiere adresse A trouvee.
pub fn parse_response(buf: &[u8], id: u16) -> Option<Ipv4Addr> {
    if buf.len() < 12 { return None; }
    let rid = ((buf[0] as u16) << 8) | buf[1] as u16;
    if rid != id { return None; }
    let qd = ((buf[4] as usize) << 8) | buf[5] as usize;
    let an = ((buf[6] as usize) << 8) | buf[7] as usize;
    let mut pos = 12;
    // Saute les questions.
    for _ in 0..qd {
        pos = skip_name(buf, pos)?;
        pos += 4; // QTYPE + QCLASS
    }
    // Parcourt les reponses.
    for _ in 0..an {
        pos = skip_name(buf, pos)?;
        if pos + 10 > buf.len() { return None; }
        let rtype = ((buf[pos] as u16) << 8) | buf[pos + 1] as u16;
        let rdlen = ((buf[pos + 8] as usize) << 8) | buf[pos + 9] as usize;
        pos += 10;
        if pos + rdlen > buf.len() { return None; }
        if rtype == 1 && rdlen == 4 {
            return Some([buf[pos], buf[pos + 1], buf[pos + 2], buf[pos + 3]]);
        }
        pos += rdlen;
    }
    None
}
