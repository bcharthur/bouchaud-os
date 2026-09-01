#!/usr/bin/env python3
"""Manifeste de publication : quoi, quelle version, et quelle empreinte.

Une image qui boote sans qu'on sache CE qui a ete construit n'est pas une
publication, c'est un artefact. Le manifeste repond a trois questions qu'on se
pose toujours trop tard : de quel commit vient cette image, quels fichiers la
composent, et est-ce bien celle qui a ete testee.

L'empreinte est un SHA-256 par fichier, plus une empreinte de l'ensemble. Deux
constructions du meme arbre doivent donner le meme manifeste ; si elles n'y
arrivent pas, la difference est nommee fichier par fichier au lieu d'etre un
soupcon.
"""

import argparse
import hashlib
import json
import subprocess
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent.parent


def empreinte(chemin: Path) -> str:
    digest = hashlib.sha256()
    with chemin.open("rb") as flux:
        for bloc in iter(lambda: flux.read(1 << 20), b""):
            digest.update(bloc)
    return digest.hexdigest()


def git(*args: str) -> str:
    try:
        return subprocess.run(("git", *args), cwd=RACINE, capture_output=True,
                              text=True, check=True).stdout.strip()
    except (subprocess.CalledProcessError, FileNotFoundError):
        return ""


def main() -> int:
    parseur = argparse.ArgumentParser()
    parseur.add_argument("artefacts", nargs="+", type=Path,
                         help="fichiers a publier (bootimage, outils, ...)")
    parseur.add_argument("--version", default="", help="etiquette de version")
    parseur.add_argument("--sortie", type=Path, default=RACINE / "release-manifest.json")
    parseur.add_argument("--verifie", action="store_true",
                         help="compare aux empreintes d'un manifeste existant")
    options = parseur.parse_args()

    fichiers = []
    for artefact in options.artefacts:
        if not artefact.exists():
            print(f"artefact absent : {artefact}")
            return 1
        fichiers.append({
            "chemin": artefact.relative_to(RACINE).as_posix()
                      if artefact.is_absolute() and RACINE in artefact.parents
                      else artefact.as_posix(),
            "octets": artefact.stat().st_size,
            "sha256": empreinte(artefact),
        })
    fichiers.sort(key=lambda f: f["chemin"])

    ensemble = hashlib.sha256(
        "".join(f"{f['sha256']}  {f['chemin']}\n" for f in fichiers).encode()
    ).hexdigest()

    manifeste = {
        "version": options.version or git("describe", "--tags", "--always", "--dirty"),
        "commit": git("rev-parse", "HEAD"),
        "branche": git("rev-parse", "--abbrev-ref", "HEAD"),
        "arbre_propre": git("status", "--porcelain") == "",
        "sha256_ensemble": ensemble,
        "fichiers": fichiers,
    }

    if options.verifie:
        if not options.sortie.exists():
            print(f"aucun manifeste a comparer : {options.sortie}")
            return 1
        ancien = json.loads(options.sortie.read_text(encoding="utf-8"))
        if ancien.get("sha256_ensemble") == ensemble:
            print(f"ok  reproductible : {ensemble[:16]}")
            return 0
        # Nommer la difference : « ce n'est pas reproductible » n'aide personne.
        avant = {f["chemin"]: f["sha256"] for f in ancien.get("fichiers", [])}
        for fichier in fichiers:
            attendu = avant.get(fichier["chemin"])
            if attendu is None:
                print(f"  nouveau  {fichier['chemin']}")
            elif attendu != fichier["sha256"]:
                print(f"  differe  {fichier['chemin']}")
                print(f"           attendu {attendu[:16]} obtenu {fichier['sha256'][:16]}")
        for chemin in sorted(set(avant) - {f["chemin"] for f in fichiers}):
            print(f"  absent   {chemin}")
        return 1

    options.sortie.write_text(json.dumps(manifeste, indent=2) + "\n", encoding="utf-8")
    print(f"manifeste : {options.sortie.name}  version={manifeste['version']}  "
          f"ensemble={ensemble[:16]}  ({len(fichiers)} fichier(s))")
    return 0


if __name__ == "__main__":
    sys.exit(main())
