//! X.509 : parsing de certificat et verification de signature (chainage).

use super::asn1::{self, Der};
use super::{rsa, p256};
use alloc::string::String;
use alloc::vec::Vec;

// --- OID (contenu DER, sans le tag) ---
const OID_RSA_ENCRYPTION: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01];
const OID_SHA256_RSA: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0b];
const OID_RSA_PSS: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0a];
const OID_EC_PUBKEY: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01];
const OID_P256: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07];
const OID_ECDSA_SHA256: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02];
const OID_SAN: &[u8] = &[0x55, 0x1d, 0x11];
const OID_BASIC_CONSTRAINTS: &[u8] = &[0x55, 0x1d, 0x13];

#[derive(Clone, Copy, PartialEq)]
pub enum SigAlg {
    RsaPkcs1Sha256,
    RsaPss,
    EcdsaP256Sha256,
    Unknown,
}

#[derive(Clone)]
pub enum PubKey {
    Rsa { n: Vec<u8>, e: Vec<u8> },
    EcP256 { point: Vec<u8> },
    Unknown,
}

/// Certificat parse (donnees possedees).
pub struct Certificate {
    pub tbs: Vec<u8>,
    pub sig_alg: SigAlg,
    pub signature: Vec<u8>,
    pub subject: Vec<u8>,
    pub issuer: Vec<u8>,
    pub pubkey: PubKey,
    pub not_before: u64,
    pub not_after: u64,
    pub san_dns: Vec<String>,
    pub is_ca: bool,
}

fn alg_from_oid(oid: &[u8]) -> SigAlg {
    if oid == OID_SHA256_RSA { SigAlg::RsaPkcs1Sha256 }
    else if oid == OID_RSA_PSS { SigAlg::RsaPss }
    else if oid == OID_ECDSA_SHA256 { SigAlg::EcdsaP256Sha256 }
    else { SigAlg::Unknown }
}

// Convertit un UTCTime/GeneralizedTime en entier comparable AAAAMMJJhhmmss.
fn parse_time(d: &Der) -> u64 {
    let s = d.content;
    let digit = |c: u8| (c - b'0') as u64;
    let two = |i: usize| -> u64 {
        if i + 1 < s.len() { digit(s[i]) * 10 + digit(s[i + 1]) } else { 0 }
    };
    if d.tag == asn1::TAG_UTCTIME && s.len() >= 12 {
        // YYMMDDHHMMSSZ : pivot 2000.
        let yy = two(0);
        let year = if yy >= 50 { 1900 + yy } else { 2000 + yy };
        year * 10_000_000_000 + two(2) * 100_000_000 + two(4) * 1_000_000
            + two(6) * 10_000 + two(8) * 100 + two(10)
    } else if s.len() >= 14 {
        // YYYYMMDDHHMMSSZ
        let year = two(0) * 100 + two(2);
        year * 10_000_000_000 + two(4) * 100_000_000 + two(6) * 1_000_000
            + two(8) * 10_000 + two(10) * 100 + two(12)
    } else {
        0
    }
}

fn parse_pubkey(spki: &Der) -> PubKey {
    // SubjectPublicKeyInfo ::= SEQUENCE { algorithm AlgorithmIdentifier, key BIT STRING }
    let mut it = spki.children();
    let alg = match it.next() { Some(a) => a, None => return PubKey::Unknown };
    let key_bs = match it.next() { Some(k) => k, None => return PubKey::Unknown };
    let alg_oid = match alg.first_child() { Some(o) => o, None => return PubKey::Unknown };

    // BIT STRING : premier octet = bits inutilises (0).
    if key_bs.content.is_empty() { return PubKey::Unknown; }
    let key_bytes = &key_bs.content[1..];

    if alg_oid.content == OID_RSA_ENCRYPTION {
        // key_bytes = DER de RSAPublicKey ::= SEQUENCE { modulus INTEGER, exponent INTEGER }
        if let Some((seq, _)) = asn1::read_tag(key_bytes, asn1::TAG_SEQUENCE) {
            let mut ki = seq.children();
            if let (Some(n), Some(e)) = (ki.next(), ki.next()) {
                return PubKey::Rsa { n: n.content.to_vec(), e: e.content.to_vec() };
            }
        }
        PubKey::Unknown
    } else if alg_oid.content == OID_EC_PUBKEY {
        // verifie la courbe (P-256) si presente
        let mut ai = alg.children();
        let _ = ai.next(); // OID deja lu
        if let Some(curve) = ai.next() {
            if curve.content != OID_P256 { return PubKey::Unknown; }
        }
        PubKey::EcP256 { point: key_bytes.to_vec() }
    } else {
        PubKey::Unknown
    }
}

fn parse_extensions(tbs_children: &mut asn1::DerIter, cert: &mut Certificate) {
    // Cherche le champ [3] EXPLICIT contenant les extensions.
    for c in tbs_children.by_ref() {
        if c.tag == 0xa3 {
            // [3] -> SEQUENCE OF Extension
            if let Some(exts) = c.first_child() {
                for ext in exts.children() {
                    // Extension ::= SEQUENCE { extnID OID, critical BOOL OPTIONAL, extnValue OCTET STRING }
                    let mut ei = ext.children();
                    let oid = match ei.next() { Some(o) => o, None => continue };
                    let mut next = ei.next();
                    // saute le booleen critical s'il est present
                    if let Some(n) = next {
                        if n.tag == asn1::TAG_BOOLEAN {
                            next = ei.next();
                        }
                    }
                    let val = match next { Some(v) => v, None => continue };
                    if oid.content == OID_SAN {
                        parse_san(val.content, cert);
                    } else if oid.content == OID_BASIC_CONSTRAINTS {
                        parse_basic_constraints(val.content, cert);
                    }
                }
            }
        }
    }
}

fn parse_san(octet_content: &[u8], cert: &mut Certificate) {
    // extnValue OCTET STRING contient un SEQUENCE OF GeneralName.
    if let Some((seq, _)) = asn1::read_tag(octet_content, asn1::TAG_SEQUENCE) {
        for name in seq.children() {
            // dNSName = [2] IMPLICIT IA5String
            if name.tag == 0x82 {
                let mut s = String::new();
                for &b in name.content { s.push(b as char); }
                cert.san_dns.push(s);
            }
        }
    }
}

fn parse_basic_constraints(octet_content: &[u8], cert: &mut Certificate) {
    if let Some((seq, _)) = asn1::read_tag(octet_content, asn1::TAG_SEQUENCE) {
        if let Some(first) = seq.first_child() {
            if first.tag == asn1::TAG_BOOLEAN && !first.content.is_empty() {
                cert.is_ca = first.content[0] != 0;
            }
        }
    }
}

/// Parse un certificat DER complet.
pub fn parse(der: &[u8]) -> Option<Certificate> {
    let (cert_seq, _) = asn1::read_tag(der, asn1::TAG_SEQUENCE)?;
    let mut top = cert_seq.children();
    let tbs = top.next()?;             // tbsCertificate
    let sig_alg_id = top.next()?;      // signatureAlgorithm
    let sig_bs = top.next()?;          // signatureValue BIT STRING

    let sig_oid = sig_alg_id.first_child()?;
    let sig_alg = alg_from_oid(sig_oid.content);
    if sig_bs.content.is_empty() { return None; }
    let signature = sig_bs.content[1..].to_vec();

    let mut cert = Certificate {
        tbs: tbs.full.to_vec(),
        sig_alg,
        signature,
        subject: Vec::new(),
        issuer: Vec::new(),
        pubkey: PubKey::Unknown,
        not_before: 0,
        not_after: u64::MAX,
        san_dns: Vec::new(),
        is_ca: false,
    };

    // Parcours du TBSCertificate.
    let mut tc = tbs.children();
    let first = tc.next()?;
    // version [0] EXPLICIT optionnel : s'il est present, le serialNumber suit.
    if first.tag == 0xa0 {
        let _serial = tc.next()?;
    }
    // (sinon `first` etait deja le serialNumber)
    let _signature_inner = tc.next()?; // AlgorithmIdentifier
    let issuer = tc.next()?;           // issuer Name
    let validity = tc.next()?;         // validity
    let subject = tc.next()?;          // subject Name
    let spki = tc.next()?;             // subjectPublicKeyInfo

    cert.issuer = issuer.full.to_vec();
    cert.subject = subject.full.to_vec();

    let mut vi = validity.children();
    if let (Some(nb), Some(na)) = (vi.next(), vi.next()) {
        cert.not_before = parse_time(&nb);
        cert.not_after = parse_time(&na);
    }
    cert.pubkey = parse_pubkey(&spki);

    parse_extensions(&mut tc, &mut cert);
    Some(cert)
}

/// Verifie que `child` est signe par la cle publique de `issuer`.
pub fn verify_signed_by(child: &Certificate, issuer_pubkey: &PubKey) -> bool {
    match (child.sig_alg, issuer_pubkey) {
        (SigAlg::RsaPkcs1Sha256, PubKey::Rsa { n, e }) => {
            let key = rsa::RsaPubKey::new(n, e);
            rsa::verify_pkcs1_sha256(&key, &child.tbs, &child.signature)
        }
        (SigAlg::RsaPss, PubKey::Rsa { n, e }) => {
            let key = rsa::RsaPubKey::new(n, e);
            rsa::verify_pss_sha256(&key, &child.tbs, &child.signature)
        }
        (SigAlg::EcdsaP256Sha256, PubKey::EcP256 { point }) => {
            // signature = SEQUENCE { r INTEGER, s INTEGER }
            if let Some((seq, _)) = asn1::read_tag(&child.signature, asn1::TAG_SEQUENCE) {
                let mut si = seq.children();
                if let (Some(r), Some(s)) = (si.next(), si.next()) {
                    return p256::verify_ecdsa_sha256(point, &child.tbs,
                        strip0(r.content), strip0(s.content));
                }
            }
            false
        }
        _ => false,
    }
}

// Retire un eventuel octet de signe 0x00 d'un INTEGER DER.
fn strip0(b: &[u8]) -> &[u8] {
    let mut i = 0;
    while i + 1 < b.len() && b[i] == 0 { i += 1; }
    &b[i..]
}

/// Le nom d'hote correspond-il a un SAN (gere les jokers *.exemple.com) ?
pub fn matches_hostname(cert: &Certificate, host: &str) -> bool {
    for san in &cert.san_dns {
        if san_match(san, host) { return true; }
    }
    false
}

fn san_match(pattern: &str, host: &str) -> bool {
    if let Some(rest) = pattern.strip_prefix("*.") {
        // joker : *.exemple.com correspond a un seul label a gauche.
        if let Some(dot) = host.find('.') {
            return eq_ci(&host[dot + 1..], rest);
        }
        return false;
    }
    eq_ci(pattern, host)
}

fn eq_ci(a: &str, b: &str) -> bool {
    if a.len() != b.len() { return false; }
    a.bytes().zip(b.bytes()).all(|(x, y)| x.to_ascii_lowercase() == y.to_ascii_lowercase())
}
