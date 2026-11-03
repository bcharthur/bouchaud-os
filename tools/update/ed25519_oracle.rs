//! Exposes the PRODUCTION Ed25519 verifier (system/update-core/src/ed25519.rs,
//! included unchanged) to tools/update/ed25519_adversarial.py.
//!
//!   rustc --edition 2021 -O tools/update/ed25519_oracle.rs -o /tmp/ed25519-oracle
//!
//! stdin: one case per line, "pk_hex msg_hex sig_hex" (msg may be "-" for
//! empty). stdout: "1" (accepted) or "0" (rejected) per line.

#[path = "../../system/update-core/src/ed25519.rs"]
mod ed25519;

use std::io::{BufRead, Write};

fn hex(s: &str) -> Option<Vec<u8>> {
    if s == "-" { return Some(Vec::new()); }
    if s.len() % 2 != 0 { return None; }
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()).collect()
}

fn main() {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        let p: Vec<&str> = line.split_whitespace().collect();
        let ok = (|| {
            let pk = hex(p.first()?)?;
            let msg = hex(p.get(1)?)?;
            let sig = hex(p.get(2)?)?;
            if pk.len() != 32 || sig.len() != 64 { return Some(false); }
            let mut k = [0u8; 32];
            k.copy_from_slice(&pk);
            let mut s = [0u8; 64];
            s.copy_from_slice(&sig);
            Some(ed25519::verifie(&k, &msg, &s))
        })().unwrap_or(false);
        writeln!(out, "{}", if ok { 1 } else { 0 }).unwrap();
    }
}
