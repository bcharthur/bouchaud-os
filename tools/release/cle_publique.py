#!/usr/bin/env python3
"""BOUCHAUD_CLES_PRODUCTION_V1 -- extrait la moitie PUBLIQUE d'une cle Ed25519.

    cle_publique.py CLE.pem [--nom release-2026-11]

Accepte une cle privee OU publique au format PEM. N'affiche QUE la cle
publique (64 hex) et la ligne a ajouter dans src/kernel/update/cles_production.rs.
A lancer HORS LIGNE, sur la machine qui detient la cle privee ; ne jamais
copier la cle privee dans le depot, sur un runner ou sur le Trigkey.
"""
from __future__ import annotations

import argparse
import re
import subprocess
import sys

SPKI_ED25519 = bytes.fromhex("302a300506032b6570032100")


def publique(pem: str) -> str:
    for args in (["openssl", "pkey", "-in", pem, "-pubout", "-outform", "DER"],
                 ["openssl", "pkey", "-pubin", "-in", pem, "-outform", "DER"]):
        r = subprocess.run(args, capture_output=True)
        if r.returncode == 0 and len(r.stdout) == 44 and r.stdout.startswith(SPKI_ED25519):
            return r.stdout[len(SPKI_ED25519):].hex()
    raise ValueError("pas une cle Ed25519 lisible")


def main(argv: list[str]) -> int:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("pem")
    p.add_argument("--nom", required=True, help="ex. release-2026-11")
    a = p.parse_args(argv[1:])
    if not re.fullmatch(r"[a-z0-9._-]{1,32}", a.nom):
        print("CLE_REFUSEE raison=nom_invalide ([a-z0-9._-]{1,32})", file=sys.stderr)
        return 2
    try:
        h = publique(a.pem)
    except ValueError as e:
        print(f"CLE_REFUSEE raison={e}", file=sys.stderr)
        return 2
    print(h)
    print(f'    ("{a.nom}", "{h}"),')
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
