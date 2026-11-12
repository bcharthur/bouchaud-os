#!/usr/bin/env python3
"""BOUCHAUD_RELEASE_V1 -- verifie un repertoire de release AVANT publication.

    verifie_artefacts_release.py DIR [--production]

Fail closed (code 2, `ARTEFACTS_REFUSES raison=...`). Exige :
  - SHA256SUMS couvre EXACTEMENT les fichiers du repertoire (rien de publie
    hors empreinte, rien d'annonce d'absent), et chaque empreinte est juste ;
  - disque-ab.img : son empreinte == disque-ab.img.sha256 == manifest-ab.json
    (l'image publiee est celle qui a ete construite et demarree sous QEMU,
    pas une image modifiee par l'essai -- revue #306) ;
  - l'image de slot == release.json == ligne image_sha256/image_octets du
    manifeste signe ; release.json nomme l'empreinte exacte du manifeste ;
  - preuve QEMU : meme commit que la construction et que release.json,
    qemu_boot PASS, physical NOT_RUN ;
  - la signature du manifeste verifie avec une cle ; avec --production, avec
    une cle ENGAGEE dans src/kernel/update/cles_production.rs, et la release
    ne se declare pas `developpement`.
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fabrique_release as fr  # noqa: E402

ATTENDUS = {"manifest", "manifest.sig", "release.json", "SHA256SUMS", "system-slot.img",
            "disque-ab.img", "disque-ab.img.sha256", "manifest-ab.json", "qemu-boot-evidence.json"}


class Refus(Exception):
    pass


def verifie(d: Path, production: bool, cles_texte: str | None = None) -> dict:
    presents = {p.name for p in d.iterdir()}
    if presents != ATTENDUS:
        raise Refus(f"fichiers_inattendus manquants={sorted(ATTENDUS - presents)} en_trop={sorted(presents - ATTENDUS)}")
    if any((d / n).is_symlink() for n in presents):
        raise Refus("lien_symbolique")
    sommes = {}
    for ligne in (d / "SHA256SUMS").read_text().splitlines():
        h, _, nom = ligne.partition("  ")
        if not nom or nom in sommes:
            raise Refus(f"sha256sums_ligne_invalide {ligne!r}")
        sommes[nom] = h
    if set(sommes) != ATTENDUS - {"SHA256SUMS"}:
        raise Refus(f"sha256sums_incomplet {sorted((ATTENDUS - {'SHA256SUMS'}) ^ set(sommes))}")
    reel = {n: fr.sha256_fichier(d / n) for n in sommes}
    faux = sorted(n for n in sommes if sommes[n] != reel[n])
    if faux:
        raise Refus(f"empreinte_fausse {faux}")

    disque = reel["disque-ab.img"]
    if (d / "disque-ab.img.sha256").read_text().split()[0] != disque:
        raise Refus("disque_modifie_depuis_la_construction (disque-ab.img.sha256)")
    ab = json.loads((d / "manifest-ab.json").read_text())
    if ab.get("image", {}).get("sha256") != disque:
        raise Refus("disque_modifie_depuis_la_construction (manifest-ab.json)")

    rel = json.loads((d / "release.json").read_text())
    texte = (d / "manifest").read_bytes()
    lignes = dict(l.split("=", 1) for l in texte.decode().splitlines()[1:])
    slot, taille = reel["system-slot.img"], (d / "system-slot.img").stat().st_size
    if not (rel["image"]["sha256"] == slot == lignes.get("image_sha256")):
        raise Refus("image_de_slot_differente_du_manifeste")
    if not (rel["image"]["octets"] == taille == int(lignes.get("image_octets", "-1"))):
        raise Refus("taille_de_slot_differente_du_manifeste")
    if rel["manifeste"]["sha256"] != reel["manifest"]:
        raise Refus("release_json_ne_nomme_pas_ce_manifeste")

    preuve = json.loads((d / "qemu-boot-evidence.json").read_text())
    if preuve.get("qemu_boot") != "PASS" or preuve.get("physical") != "NOT_RUN":
        raise Refus("preuve_qemu_absente_ou_pretend_physique")
    if not (preuve.get("build_sha") == ab.get("commit") == rel.get("commit")):
        raise Refus("commits_differents (construction, essai QEMU, release)")

    signature = (d / "manifest.sig").read_bytes()
    if production:
        if rel["signature"].get("developpement") is not False:
            raise Refus("release_de_developpement")
        cles = fr.cles_production(cles_texte if cles_texte is not None
                                  else fr.CLES_PRODUCTION.read_text(encoding="utf-8"))
        nom = next((n for n, h in cles if fr.verifie(h, texte, signature)), None)
        if nom is None:
            raise Refus("signature_ne_verifie_avec_aucune_cle_engagee")
        if nom != rel["signature"].get("cle"):
            raise Refus("cle_annoncee_differente")
    return rel


def main(argv: list[str]) -> int:
    if len(argv) not in (2, 3) or (len(argv) == 3 and argv[2] != "--production"):
        print(__doc__, file=sys.stderr)
        return 2
    try:
        rel = verifie(Path(argv[1]), len(argv) == 3)
    except (Refus, OSError, ValueError, KeyError, TypeError, json.JSONDecodeError, fr.Refus) as e:
        print(f"ARTEFACTS_REFUSES raison={e}", file=sys.stderr)
        return 2
    print(f"ARTEFACTS_OK version={rel['version']} sequence={rel['sequence']} cle={rel['signature']['cle']}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
