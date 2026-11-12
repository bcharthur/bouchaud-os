#!/usr/bin/env python3
"""BOUCHAUD_RELEASE_V1 -- fabrique une release Trigkey signee, ou refuse.

    fabrique_release.py --image SLOT.img --version V --sequence N --canal stable|testing
                        --commit SHA40 --sortie DIR
                        [--sequence-precedente M] [--cle PRIVEE.pem | --cle-dev PRIVEE.pem]
                        [--materiel x86_64-uefi] [--essais 3]

Produit dans DIR (qui doit etre vide ou absent : une release n'est jamais
reecrite) :
  manifest        le manifeste v2 CANONIQUE (system/update-core/src/manifeste.rs)
  manifest.sig    sa signature Ed25519 (64 octets)
  release.json    descripteur : version, sequence, canal, commit, materiel,
                  empreintes, cle, informations de rollback
  SHA256SUMS      empreintes de tous les fichiers ci-dessus et de l'image

Refus (code 2, `RELEASE_REFUSEE raison=...`) :
  - sequence <= sequence-precedente (monotonie : pas de retrograde, pas de
    reutilisation d'un numero) ;
  - --cle sans cle de production engagee dans src/kernel/update/cles_production.rs,
    ou signature qui ne verifie avec AUCUNE d'entre elles (mauvais secret) ;
  - ni --cle ni --cle-dev ;
  - repertoire de sortie non vide.
--cle-dev (bancs seulement) : signe avec une cle de developpement, que le
descripteur declare `developpement: true` ; le workflow de publication la
refuse.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent.parent
CLES_PRODUCTION = RACINE / "src/kernel/update/cles_production.rs"
MATERIEL_RE = re.compile(r"^[a-z0-9_-]{1,32}$")
VERSION_RE = re.compile(r"^[A-Za-z0-9._+-]{1,64}$")
# En-tete SPKI DER d'une cle publique Ed25519 (RFC 8410), suivi des 32 octets.
SPKI_ED25519 = bytes.fromhex("302a300506032b6570032100")


class Refus(Exception):
    pass


def manifeste_v2(version: str, sequence: int, materiel: str, sha256: str, octets: int, essais: int) -> bytes:
    """La forme canonique v2, octet pour octet (manifeste.rs)."""
    if not VERSION_RE.match(version):
        raise Refus("version_invalide")
    if sequence <= 0 or sequence >= 10 ** 19:
        raise Refus("sequence_invalide")
    if not MATERIEL_RE.match(materiel):
        raise Refus("materiel_invalide")
    if not re.fullmatch(r"[0-9a-f]{64}", sha256):
        raise Refus("empreinte_invalide")
    if octets <= 0 or octets >= 10 ** 19:
        raise Refus("taille_invalide")
    if not 1 <= essais <= 9:
        raise Refus("essais_invalides")
    return (f"bouchaud-manifeste-v2\nversion={version}\nsequence={sequence}\nmateriel={materiel}\n"
            f"image_sha256={sha256}\nimage_octets={octets}\nessais={essais}\n").encode()


def cles_production(texte: str) -> list[tuple[str, str]]:
    """Les cles publiques ENGAGEES (hors commentaires)."""
    sans = "\n".join(l.split("//", 1)[0] for l in texte.splitlines())
    return re.findall(r'\(\s*"([^"]+)"\s*,\s*"([0-9a-f]{64})"\s*\)', sans)


def signe(cle_pem: Path, donnees: bytes) -> bytes:
    with tempfile.TemporaryDirectory() as d:
        m = Path(d, "m")
        m.write_bytes(donnees)
        r = subprocess.run(["openssl", "pkeyutl", "-sign", "-rawin", "-inkey", str(cle_pem), "-in", str(m)],
                           capture_output=True)
        if r.returncode != 0 or len(r.stdout) != 64:
            raise Refus("signature_impossible")
        return r.stdout


def verifie(publique_hex: str, donnees: bytes, signature: bytes) -> bool:
    with tempfile.TemporaryDirectory() as d:
        pub = Path(d, "pub.der")
        pub.write_bytes(SPKI_ED25519 + bytes.fromhex(publique_hex))
        m, s = Path(d, "m"), Path(d, "s")
        m.write_bytes(donnees)
        s.write_bytes(signature)
        r = subprocess.run(["openssl", "pkeyutl", "-verify", "-rawin", "-pubin", "-keyform", "DER",
                            "-inkey", str(pub), "-in", str(m), "-sigfile", str(s)], capture_output=True)
        return r.returncode == 0


def publique_de(cle_pem: Path) -> str:
    r = subprocess.run(["openssl", "pkey", "-in", str(cle_pem), "-pubout", "-outform", "DER"], capture_output=True)
    if r.returncode != 0 or not r.stdout.startswith(SPKI_ED25519):
        raise Refus("cle_illisible")
    return r.stdout[len(SPKI_ED25519):].hex()


def sha256_fichier(p: Path) -> str:
    h = hashlib.sha256()
    with p.open("rb") as f:
        for b in iter(lambda: f.read(1 << 20), b""):
            h.update(b)
    return h.hexdigest()


def fabrique(a: argparse.Namespace) -> Path:
    if a.sequence_precedente is not None and a.sequence <= a.sequence_precedente:
        raise Refus(f"sequence_non_croissante precedente={a.sequence_precedente} demandee={a.sequence}")
    if a.canal not in ("stable", "testing"):
        raise Refus("canal_invalide")
    if not re.fullmatch(r"[0-9a-f]{40}", a.commit):
        raise Refus("commit_invalide")
    sortie = Path(a.sortie)
    if sortie.exists() and any(sortie.iterdir()):
        raise Refus("sortie_non_vide (une release n'est jamais reecrite)")
    image = Path(a.image)
    if not image.is_file() or image.stat().st_size == 0:
        raise Refus("image_absente")
    empreinte, octets = sha256_fichier(image), image.stat().st_size
    texte = manifeste_v2(a.version, a.sequence, a.materiel, empreinte, octets, a.essais)

    if a.cle:
        cles = cles_production(CLES_PRODUCTION.read_text(encoding="utf-8"))
        if not cles:
            raise Refus("aucune_cle_de_production_engagee (src/kernel/update/cles_production.rs)")
        signature = signe(Path(a.cle), texte)
        nom = next((n for n, h in cles if verifie(h, texte, signature)), None)
        if nom is None:
            raise Refus("signature_ne_verifie_avec_aucune_cle_engagee (mauvais secret ?)")
        developpement = False
    elif a.cle_dev:
        signature = signe(Path(a.cle_dev), texte)
        if not verifie(publique_de(Path(a.cle_dev)), texte, signature):
            raise Refus("signature_dev_invalide")
        nom, developpement = "dev-banc", True
    else:
        raise Refus("aucune_cle (--cle ou --cle-dev)")

    sortie.mkdir(parents=True, exist_ok=True)
    (sortie / "manifest").write_bytes(texte)
    (sortie / "manifest.sig").write_bytes(signature)
    descripteur = {
        "schema": "bouchaud-release/1",
        "version": a.version,
        "sequence": a.sequence,
        "canal_publication": a.canal,
        "commit": a.commit,
        "materiel": a.materiel,
        "image": {"fichier": image.name, "octets": octets, "sha256": empreinte},
        "manifeste": {"fichier": "manifest", "sha256": hashlib.sha256(texte).hexdigest(), "format": "v2"},
        "signature": {"fichier": "manifest.sig", "algorithme": "ed25519", "cle": nom, "developpement": developpement},
        "rollback": {"essais": a.essais, "sequence_precedente": a.sequence_precedente,
                     "plancher_apres_commit": a.sequence},
    }
    (sortie / "release.json").write_text(json.dumps(descripteur, indent=2, sort_keys=True) + "\n")
    sommes = {f: sha256_fichier(sortie / f) for f in ("manifest", "manifest.sig", "release.json")}
    sommes[image.name] = empreinte
    (sortie / "SHA256SUMS").write_text("".join(f"{h}  {f}\n" for f, h in sorted(sommes.items())))
    return sortie


def main(argv: list[str]) -> int:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--image", required=True)
    p.add_argument("--version", required=True)
    p.add_argument("--sequence", type=int, required=True)
    p.add_argument("--canal", required=True)
    p.add_argument("--commit", required=True)
    p.add_argument("--sortie", required=True)
    p.add_argument("--sequence-precedente", type=int)
    p.add_argument("--materiel", default="x86_64-uefi")
    p.add_argument("--essais", type=int, default=3)
    g = p.add_mutually_exclusive_group()
    g.add_argument("--cle")
    g.add_argument("--cle-dev")
    a = p.parse_args(argv[1:])
    try:
        sortie = fabrique(a)
    except Refus as e:
        print(f"RELEASE_REFUSEE raison={e}", file=sys.stderr)
        return 2
    d = json.loads((sortie / "release.json").read_text())
    print(f"RELEASE_FABRIQUEE version={d['version']} sequence={d['sequence']} canal={d['canal_publication']} "
          f"cle={d['signature']['cle']} developpement={int(d['signature']['developpement'])} "
          f"image_sha256={d['image']['sha256'][:16]}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
