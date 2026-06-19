//! Parseur ASN.1 / DER minimal (lecture seule) pour X.509.

/// Classes de tags ASN.1 courants.
pub const TAG_BOOLEAN: u8 = 0x01;
pub const TAG_INTEGER: u8 = 0x02;
pub const TAG_BIT_STRING: u8 = 0x03;
pub const TAG_OCTET_STRING: u8 = 0x04;
pub const TAG_NULL: u8 = 0x05;
pub const TAG_OID: u8 = 0x06;
pub const TAG_UTF8STRING: u8 = 0x0c;
pub const TAG_PRINTABLESTRING: u8 = 0x13;
pub const TAG_IA5STRING: u8 = 0x16;
pub const TAG_UTCTIME: u8 = 0x17;
pub const TAG_GENERALIZEDTIME: u8 = 0x18;
pub const TAG_SEQUENCE: u8 = 0x30;
pub const TAG_SET: u8 = 0x31;

/// Un element DER decode : tag, contenu, et la portion complete (tag+len+valeur).
#[derive(Clone, Copy)]
pub struct Der<'a> {
    pub tag: u8,
    pub content: &'a [u8],
    pub full: &'a [u8],
}

/// Lit un seul element TLV au debut de `data`. Renvoie (element, reste).
pub fn read(data: &[u8]) -> Option<(Der<'_>, &[u8])> {
    if data.len() < 2 { return None; }
    let tag = data[0];
    let first = data[1];
    let (len, header) = if first & 0x80 == 0 {
        (first as usize, 2)
    } else {
        let num = (first & 0x7f) as usize;
        if num == 0 || num > 4 || data.len() < 2 + num { return None; }
        let mut l = 0usize;
        for i in 0..num {
            l = (l << 8) | data[2 + i] as usize;
        }
        (l, 2 + num)
    };
    if data.len() < header + len { return None; }
    let content = &data[header..header + len];
    let full = &data[..header + len];
    Some((Der { tag, content, full }, &data[header + len..]))
}

/// Lit un element en exigeant un tag precis.
pub fn read_tag(data: &[u8], tag: u8) -> Option<(Der<'_>, &[u8])> {
    let (e, rest) = read(data)?;
    if e.tag != tag { return None; }
    Some((e, rest))
}

impl<'a> Der<'a> {
    /// Itere sur les sous-elements (pour SEQUENCE/SET).
    pub fn children(&self) -> DerIter<'a> {
        DerIter { rest: self.content }
    }

    /// Premier enfant.
    pub fn first_child(&self) -> Option<Der<'a>> {
        self.children().next()
    }
}

pub struct DerIter<'a> {
    rest: &'a [u8],
}

impl<'a> Iterator for DerIter<'a> {
    type Item = Der<'a>;
    fn next(&mut self) -> Option<Der<'a>> {
        if self.rest.is_empty() { return None; }
        let (e, rest) = read(self.rest)?;
        self.rest = rest;
        Some(e)
    }
}
