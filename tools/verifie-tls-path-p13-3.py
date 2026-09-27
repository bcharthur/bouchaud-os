#!/usr/bin/env python3
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]
ROOTS = ROOT / "src" / "net" / "security" / "tls" / "roots.rs"
VALIDATE = ROOT / "src" / "net" / "security" / "tls" / "validate.rs"


def fail(msg: str) -> None:
    print(f"P13_3_TLS_PATH_FAIL: {msg}", file=sys.stderr)
    raise SystemExit(1)

roots = ROOTS.read_text(encoding="utf-8")
validate = VALIDATE.read_text(encoding="utf-8")

checks = {
    "roots marker": "BOUCHAUD_P13_3_EQUIVALENT_TRUST_ANCHOR_V1" in roots,
    "equivalent anchor fn": "pub fn find_equivalent_anchor(cert: &Certificate)" in roots,
    "subject equality": "root.subject == cert.subject" in roots,
    "SPKI/public key equality": "meme_cle_publique(&root.pubkey, &cert.pubkey)" in roots,
    "RSA key equality": "an == bn && ae == be" in roots,
    "P256 key equality": "PubKey::EcP256" in roots and "ap == bp" in roots,
    "P384 key equality": "PubKey::EcP384" in roots,
    "validate marker": "BOUCHAUD_P13_3_CROSS_SIGNED_ANCHOR_V1" in validate,
    "issuer subject hardening": "certs[i].issuer != certs[i + 1].subject" in validate,
    "equivalent anchor used": "roots::find_equivalent_anchor(last).is_some()" in validate,
    "trusted only after anchor": 'res.anchor = Some("magasin:subject+spki")' in validate,
    "legacy issuer path preserved": "match roots::find_issuer_for(last)" in validate,
}

for name, ok in checks.items():
    if not ok:
        fail(name)

# Le patch ne doit pas faire confiance a AAA par ajout opportuniste au magasin.
if 'include_bytes!("ca/AAA' in roots or 'AAA_Certificate' in roots:
    fail("AAA ne doit pas etre ajoutee comme trust anchor par P13.3")

print("P13_3_TLS_PATH_VERIFY_OK")
