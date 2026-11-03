//! Build a META partition image (1 MiB) from the production encoder.
//!
//!   rustc --edition 2021 -O tools/update/meta.rs -o /tmp/bo-meta
//!   /tmp/bo-meta SPEC OUT
//!
//! SPEC:
//!   a-bon            A Good (fresh install), B empty
//!   b-candidat:N     A Good, B Candidate with N boot attempts
//!   b-bon            A Good, B Good, active A   (manual-rollback scenario)
//!   corrompu:0|1     as b-candidat:3, copy 0 (or 1) overwritten with garbage
//!   corrompu:2       both copies garbage: the loader must require recovery
//!
//! Copy 0 is at block 0, copy 1 at block 1 (512-byte blocks). The encoder is
//! system/update-core/src/ab.rs itself, included, not re-implemented.
#[path = "../../system/update-core/src/ab.rs"]
#[allow(dead_code)]
mod ab;
use ab::*;

const MIO: usize = 1024 * 1024;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 && args.len() != 5 { eprintln!("usage: meta SPEC OUT [SHA256_A SHA256_B]"); std::process::exit(2); }
    let (spec, out) = (&args[1], &args[2]);
    // Optional: the REAL digests of the slot images (recovery `verifie`).
    let hex = |h: &str| -> [u8; 32] {
        let mut o = [0u8; 32];
        for i in 0..32 { o[i] = u8::from_str_radix(&h[2 * i..2 * i + 2], 16).expect("sha256 hex"); }
        o
    };
    let img_a = if args.len() == 5 { hex(&args[3]) } else { [0xA1u8; 32] };
    let img_b = if args.len() == 5 { hex(&args[4]) } else { [0xB2u8; 32] };
    let base = Enregistrement::initial(Slot::A, img_a);
    let candidat = |n: u8| base.commence_installation(Slot::B, Slot::A).unwrap()
        .marque_candidat(Slot::B, img_b, img_b, n).unwrap();
    let mut corrompre: Vec<usize> = Vec::new();
    let e = match spec.as_str() {
        "a-bon" => base,
        "b-bon" => {
            let c = candidat(1);
            let (_, apres) = c.choisit_boot();
            let ok = apres.sante_ok(Slot::B);
            ok.revient().unwrap()
        }
        s if s.starts_with("b-candidat:") => candidat(s[11..].parse().expect("N")),
        s if s.starts_with("corrompu:") => {
            match &s[9..] { "0" => corrompre.push(0), "1" => corrompre.push(1),
                            "2" => { corrompre.push(0); corrompre.push(1) }, _ => panic!("corrompu:0|1|2") }
            candidat(3)
        }
        _ => { eprintln!("SPEC inconnue: {spec}"); std::process::exit(2); }
    };
    let mut image = vec![0u8; MIO];
    let rec = encode(&e);
    image[..OCTETS].copy_from_slice(&rec);
    // Copy 1 is one sequence older when both are valid, as after a real write.
    let mut ancienne = e;
    ancienne.sequence = e.sequence.saturating_sub(1).max(1);
    let rec1 = if ancienne.sequence == e.sequence { rec } else { encode(&ancienne) };
    image[512..512 + OCTETS].copy_from_slice(&rec1);
    for c in corrompre { for b in &mut image[c * 512..c * 512 + OCTETS] { *b = 0x5A; } }
    std::fs::write(out, &image).expect("ecriture");
    let lu = selectionne(&image[..OCTETS], &image[512..512 + OCTETS]);
    println!("BOUCHAUD_META_OK spec={spec} out={out} lu={:?}", lu.map(|e| (e.sequence, e.actif, e.slot(Slot::A).etat, e.slot(Slot::B).etat)));
}
