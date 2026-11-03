//! Host proof of Ed25519 verification and SHA-512 (BOUCHAUD_SIGNATURE_V1).
//!
//!   rustc --edition 2021 --test tools/update/test_signature.rs -o /tmp/ts && /tmp/ts
//!
//! Production module included unchanged: system/update-core/src/ed25519.rs.
//! Independent references: RFC 8032 section 7.1 vectors, and OpenSSL (an
//! implementation this code shares nothing with) signing and hashing at test
//! time with keys generated on the spot -- no key is stored anywhere.

#[path = "../../system/update-core/src/ed25519.rs"]
mod ed25519;
#[path = "../../system/update-core/src/manifeste.rs"]
mod manifeste;

use manifeste::{analyse, verifie_signe, CleDeConfiance, RefusManifeste};

use std::process::Command;

fn hex(s: &str) -> Vec<u8> {
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect()
}
fn a32(v: &[u8]) -> [u8; 32] { let mut o = [0; 32]; o.copy_from_slice(v); o }
fn a64(v: &[u8]) -> [u8; 64] { let mut o = [0; 64]; o.copy_from_slice(v); o }

#[test]
fn sha512_vecteurs_connus() {
    let h = |d: &[u8]| ed25519::sha512(d).iter().map(|b| format!("{b:02x}")).collect::<String>();
    assert_eq!(h(b""), "cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e");
    assert_eq!(h(b"abc"), "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f");
}

#[test]
fn sha512_contre_openssl_a_chaque_frontiere_de_bloc() {
    let dir = std::env::temp_dir().join(format!("bo-sha512-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for n in [0usize, 1, 111, 112, 113, 127, 128, 129, 239, 240, 255, 256, 1000, 65537] {
        let d: Vec<u8> = (0..n).map(|i| (i * 31 + 7) as u8).collect();
        let f = dir.join("m");
        std::fs::write(&f, &d).unwrap();
        let out = Command::new("openssl").args(["dgst", "-sha512", "-r"]).arg(&f).output().expect("openssl");
        let attendu = String::from_utf8(out.stdout).unwrap();
        let obtenu: String = ed25519::sha512(&d).iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(&attendu[..128], obtenu, "n={n}");
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn rfc8032_vecteurs() {
    let v = [
        ("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a", "",
         "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b"),
        ("3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c", "72",
         "92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00"),
        ("fc51cd8e6218a1a38da47ed00230f0580816ed13ba3303ac5deb911548908025", "af82",
         "6291d657deec24024827e69c3abe01a30ce548a284743a445e3680d7db5ac3ac18ff9b538d16f290ae67f760984dc6594a7c15e9716ed28dc027beceea1ec40a"),
    ];
    for (pk, m, sig) in v {
        let (pk, m, sig) = (a32(&hex(pk)), hex(m), a64(&hex(sig)));
        assert!(ed25519::verifie(&pk, &m, &sig), "RFC 8032 vector rejected");
        let mut faux = m.clone();
        faux.push(0);
        assert!(!ed25519::verifie(&pk, &faux, &sig));
    }
}

struct Cle { dir: std::path::PathBuf, publique: [u8; 32] }

fn cle_openssl() -> Cle {
    let dir = std::env::temp_dir().join(format!("bo-ed25519-{}-{}", std::process::id(), rand_u32()));
    std::fs::create_dir_all(&dir).unwrap();
    let k = dir.join("k.pem");
    assert!(Command::new("openssl").args(["genpkey", "-algorithm", "ed25519", "-out"]).arg(&k).status().unwrap().success());
    let der = Command::new("openssl").args(["pkey", "-pubout", "-outform", "DER", "-in"]).arg(&k).output().unwrap().stdout;
    Cle { publique: a32(&der[der.len() - 32..]), dir }
}

fn signe(c: &Cle, m: &[u8]) -> [u8; 64] {
    let (mf, sf) = (c.dir.join("m"), c.dir.join("s"));
    std::fs::write(&mf, m).unwrap();
    assert!(Command::new("openssl").args(["pkeyutl", "-sign", "-rawin", "-inkey"]).arg(c.dir.join("k.pem"))
        .arg("-in").arg(&mf).arg("-out").arg(&sf).status().unwrap().success());
    a64(&std::fs::read(&sf).unwrap())
}

fn rand_u32() -> u32 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos()
}

#[test]
fn signatures_openssl_acceptees_et_toute_alteration_refusee() {
    let c = cle_openssl();
    // n = 0 is RFC 8032 vector 1: OpenSSL 3.0 pkeyutl cannot sign an empty
    // message ("Could not allocate 0 bytes for oneshot sign").
    for n in [1usize, 32, 63, 64, 127, 128, 129, 1000, 4096] {
        let m: Vec<u8> = (0..n).map(|i| (i * 7 + n) as u8).collect();
        let s = signe(&c, &m);
        assert!(ed25519::verifie(&c.publique, &m, &s), "OpenSSL signature rejected n={n}");
        for bit in 0..512 {
            let mut x = s;
            x[bit / 8] ^= 1 << (bit % 8);
            assert!(!ed25519::verifie(&c.publique, &m, &x), "flipped signature bit {bit} accepted n={n}");
        }
        for bit in 0..256 {
            let mut k = c.publique;
            k[bit / 8] ^= 1 << (bit % 8);
            assert!(!ed25519::verifie(&k, &m, &s), "flipped key bit {bit} accepted n={n}");
        }
        if n > 0 {
            for i in [0, n / 2, n - 1] {
                let mut x = m.clone();
                x[i] ^= 0x01;
                assert!(!ed25519::verifie(&c.publique, &x, &s), "altered message accepted n={n}");
            }
        }
    }
    // Another key's signature is never accepted.
    let autre = cle_openssl();
    let s = signe(&autre, b"manifeste");
    assert!(!ed25519::verifie(&c.publique, b"manifeste", &s));
    std::fs::remove_dir_all(&c.dir).ok();
    std::fs::remove_dir_all(&autre.dir).ok();
}

#[test]
fn s_superieur_ou_egal_a_l_refuse() {
    // RFC 8032: S >= L must be rejected even if the equation would hold
    // (S + L is the classic malleated signature).
    let c = cle_openssl();
    let m = b"malleabilite";
    let s = signe(&c, m);
    let l: [u8; 32] = [0xed, 0xd3, 0xf5, 0x5c, 0x1a, 0x63, 0x12, 0x58, 0xd6, 0x9c, 0xf7, 0xa2, 0xde, 0xf9,
                       0xde, 0x14, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x10];
    let mut x = s;
    let mut retenue = 0u16;
    for i in 0..32 {
        let v = x[32 + i] as u16 + l[i] as u16 + retenue;
        x[32 + i] = v as u8;
        retenue = v >> 8;
    }
    assert!(ed25519::verifie(&c.publique, m, &s));
    assert!(!ed25519::verifie(&c.publique, m, &x), "S + L accepted");
    std::fs::remove_dir_all(&c.dir).ok();
}

const CANONIQUE: &str = "bouchaud-manifeste-v1\nversion=bench-1.2_a+b\nimage_sha256=00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff\nimage_octets=94371840\nessais=3\n";

#[test]
fn manifeste_canonique_seul_accepte() {
    let m = analyse(CANONIQUE.as_bytes()).expect("canonical");
    assert_eq!((m.version(), m.octets, m.essais, m.empreinte[1]), ("bench-1.2_a+b", 94371840, 3, 0x11));
    let variantes = [
        CANONIQUE.replace("\n", "\r\n"),
        CANONIQUE.trim_end().to_string(),
        format!("{CANONIQUE}\n"),
        format!("{CANONIQUE}extra=1\n"),
        CANONIQUE.replace("essais=3", "essais=0"),
        CANONIQUE.replace("essais=3", "essais=10"),
        CANONIQUE.replace("image_octets=94371840", "image_octets=094371840"),
        CANONIQUE.replace("image_octets=94371840", "image_octets=0"),
        CANONIQUE.replace("aabb", "AABB"),
        CANONIQUE.replace("version=bench-1.2_a+b", "version=bench 1"),
        CANONIQUE.replace("version=bench-1.2_a+b", "version="),
        CANONIQUE.replace("bouchaud-manifeste-v1", "bouchaud-manifeste-v2"),
        CANONIQUE.replace("image_sha256=00", "image_sha256=0"),
        CANONIQUE.lines().filter(|l| !l.starts_with("essais")).map(|l| format!("{l}\n")).collect(),
    ];
    for v in variantes {
        assert_eq!(analyse(v.as_bytes()), Err(RefusManifeste::Format), "accepted: {v:?}");
    }
}

#[test]
fn chaine_de_confiance_cle_signature_manifeste() {
    let c = cle_openssl();
    let cles = [CleDeConfiance { nom: "dev-test", publique: c.publique, developpement: true }];
    let s = signe(&c, CANONIQUE.as_bytes());
    let (m, k) = verifie_signe(CANONIQUE.as_bytes(), &s, &cles).expect("trusted");
    assert_eq!((m.essais, k.nom, k.developpement), (3, "dev-test", true));
    // No trusted key configured: never trusted.
    assert_eq!(verifie_signe(CANONIQUE.as_bytes(), &s, &[]).unwrap_err(), RefusManifeste::AucuneCleDeConfiance);
    // One byte of the manifest changed (more attempts): the signature no longer holds.
    let altere = CANONIQUE.replace("essais=3", "essais=9");
    assert_eq!(verifie_signe(altere.as_bytes(), &s, &cles).unwrap_err(), RefusManifeste::SignatureNonFiable);
    // Signed by an untrusted key.
    let autre = cle_openssl();
    let s2 = signe(&autre, CANONIQUE.as_bytes());
    assert_eq!(verifie_signe(CANONIQUE.as_bytes(), &s2, &cles).unwrap_err(), RefusManifeste::SignatureNonFiable);
    // A trusted signature over NON-canonical bytes is still refused.
    let nc = format!("{CANONIQUE}extra=1\n");
    let s3 = signe(&c, nc.as_bytes());
    assert_eq!(verifie_signe(nc.as_bytes(), &s3, &cles).unwrap_err(), RefusManifeste::Format);
    assert_eq!(manifeste::cle_hex(&c.publique.iter().map(|b| format!("{b:02x}")).collect::<String>()), Some(c.publique));
    assert_eq!(manifeste::cle_hex("zz"), None);
    std::fs::remove_dir_all(&c.dir).ok();
    std::fs::remove_dir_all(&autre.dir).ok();
}

#[test]
fn encodages_pathologiques_refuses() {
    // BOUCHAUD_ED25519_STRICT_V1 regressions (tools/update/ed25519_adversarial.py
    // found them accepted, as OpenSSL 3 does; libsodium refuses them).
    let mut identite = [0u8; 32];
    identite[0] = 1;
    let mut sig_triviale = [0u8; 64];
    sig_triviale[..32].copy_from_slice(&identite); // R = identity, S = 0
    // identity key: before the fix, verified ANY message.
    assert!(!ed25519::verifie(&identite, b"", &sig_triviale));
    assert!(!ed25519::verifie(&identite, b"n'importe quel manifeste", &sig_triviale));
    // The real forgery under the identity key: R = [r]B for any r, S = r,
    // for ANY message ([S]B = R + [k]*identity). R = B, S = 1. Only the
    // small-order KEY check refuses it (R = B is a full-order point).
    let mut sig_b = [0x66u8; 64];
    sig_b[0] = 0x58;                          // B = 5866..66
    sig_b[32..].copy_from_slice(&[0u8; 32]);
    sig_b[32] = 1;                            // S = 1
    assert!(!ed25519::verifie(&identite, b"manifeste choisi par l'attaquant", &sig_b));
    // y = 1 + p: non-canonical identity (RFC 8032 5.1.3: y >= p fails).
    let mut y_plus_p = [0xffu8; 32];
    y_plus_p[0] = 0xee;
    y_plus_p[31] = 0x7f;
    assert!(!ed25519::verifie(&y_plus_p, b"", &sig_triviale));
    // negative zero: identity with the sign bit set.
    let mut zero_negatif = identite;
    zero_negatif[31] |= 0x80;
    assert!(!ed25519::verifie(&zero_negatif, b"", &sig_triviale));
    // order-2 point (0, -1): y = p - 1.
    let mut ordre2 = [0xffu8; 32];
    ordre2[0] = 0xec;
    ordre2[31] = 0x7f;
    let mut sig2 = [0u8; 64];
    sig2[..32].copy_from_slice(&identite);
    assert!(!ed25519::verifie(&ordre2, b"", &sig2));
}

#[test]
fn manifeste_revue_entrees_hostiles() {
    // Consolidation review of manifeste.rs: ONE byte string per manifest.
    let base = CANONIQUE;
    let refuses: Vec<String> = vec![
        String::new(),
        "bouchaud-manifeste-v1\n".into(),
        // duplicate field
        base.replace("essais=3\n", "essais=3\nessais=3\n"),
        base.replace("version=bench-1.2_a+b\n", "version=bench-1.2_a+b\nversion=x\n"),
        // reordered fields
        base.replace("image_octets=94371840\nessais=3\n", "essais=3\nimage_octets=94371840\n"),
        // Unicode / control bytes in a field
        base.replace("version=bench-1.2_a+b", "version=bench-é"),
        base.replace("version=bench-1.2_a+b", "version=bench\u{0}"),
        base.replace("version=bench-1.2_a+b", "version=bench\t1"),
        // CR before LF on one line, trailing space
        base.replace("essais=3\n", "essais=3\r\n"),
        base.replace("essais=3\n", "essais=3 \n"),
        // length bounds
        base.replace("version=bench-1.2_a+b", &format!("version={}", "v".repeat(65))),
        base.replace("image_octets=94371840", "image_octets=18446744073709551616"), // 2^64: 20 digits
        base.replace("image_octets=94371840", "image_octets=99999999999999999999"),
        base.replace("image_octets=94371840", "image_octets=+94371840"),
        base.replace("image_octets=94371840", "image_octets=-1"),
        base.replace("essais=3", "essais=a"),
        base.replace("image_sha256=00", "image_sha256=g0"),
        format!("{base}\0"),
    ];
    for v in &refuses {
        assert_eq!(analyse(v.as_bytes()), Err(RefusManifeste::Format), "accepted: {v:?}");
    }
    // Upper bounds that ARE canonical parse exactly, with no overflow.
    let max19 = base.replace("image_octets=94371840", "image_octets=9999999999999999999");
    assert_eq!(analyse(max19.as_bytes()).unwrap().octets, 9_999_999_999_999_999_999);
    let v64 = base.replace("version=bench-1.2_a+b", &format!("version={}", "v".repeat(64)));
    assert_eq!(analyse(v64.as_bytes()).unwrap().version_len, 64);
    for e in 1..=9u8 {
        let m = base.replace("essais=3", &format!("essais={e}"));
        assert_eq!(analyse(m.as_bytes()).unwrap().essais, e);
    }
}
