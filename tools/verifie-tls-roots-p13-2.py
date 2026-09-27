#!/usr/bin/env python3
from __future__ import annotations
import hashlib
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]
CA = ROOT / 'src' / 'net' / 'security' / 'tls' / 'ca'
ROOTS = ROOT / 'src' / 'net' / 'security' / 'tls' / 'roots.rs'
EXPECTED = {
    'SSL_com_TLS_ECC_Root_CA_2022.der': 'c32ffd9f46f936d16c3673990959434b9ad60aafbb9e7cf33654f144cc1ba143',
    'SSL_com_TLS_RSA_Root_CA_2022.der': '8faf7d2e2cb4709bb8e0b33666bf75a5dd45b5de480f8ea8d4bfe6bebc17f2ed',
}

def fail(msg: str) -> None:
    print(f'P13_2_TLS_ROOTS_FAIL: {msg}', file=sys.stderr)
    raise SystemExit(1)

for name, want in EXPECTED.items():
    p = CA / name
    if not p.is_file():
        fail(f'certificat absent: {p}')
    data = p.read_bytes()
    got = hashlib.sha256(data).hexdigest()
    if got != want:
        fail(f'SHA256 inattendu pour {name}: {got}')
    if not data.startswith(b'0') or len(data) < 300:
        fail(f'DER manifestement invalide: {name}')

text = ROOTS.read_text(encoding='utf-8')
for name in EXPECTED:
    needle = f'include_bytes!("ca/{name}")'
    if text.count(needle) != 1:
        fail(f'reference roots.rs attendue exactement une fois: {needle}')

print('P13_2_TLS_ROOTS_VERIFY_OK')
