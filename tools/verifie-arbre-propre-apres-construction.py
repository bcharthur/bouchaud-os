#!/usr/bin/env python3
"""Garde-fou : construire une image ne doit pas salir l'arbre de travail.

# Le defaut, constate le 16 septembre 2026

`IMAGE-TRIGKEY.ps1` refuse de construire sur un arbre modifie, et c'est
volontaire : une image batie sur un arbre sale ne correspond a aucun commit,
et le releve d'un essai physique fait sur une telle image ne se rattache a
rien. La verification a fait son travail :

    Arbre de travail MODIFIE :
      ?? tools/reference/uefi-preboot-probe/Cargo.lock

    ERREUR: l'image ne correspondrait a aucun commit

Sauf que personne n'avait modifie quoi que ce soit. C'est la construction
PRECEDENTE qui avait depose ce fichier : `build-trigkey-stage2-usb.ps1`
compile le shim UEFI, et cargo ecrit un `Cargo.lock` dans le dossier du crate
meme quand on lui donne un `--target-dir` ailleurs. Le premier essai
reussissait, le second echouait, et la seule sortie apparente etait
`-QuandMeme` -- c'est-a-dire renoncer justement a la correspondance que le
controle existe pour garantir.

Un artefact de construction ne doit jamais pouvoir bloquer la construction
suivante.

# La politique du depot

Elle etait deja etablie, et ce crate y echappait simplement :

  * `Cargo.lock` d'un crate outil : SUIVI. La racine et
    `tools/reference/uefi-image-builder/` le sont. Un lock suivi fige les
    versions, donc deux machines construisent le meme binaire -- ce qui est
    tout l'interet d'un shim qui part dans une image amorcable.
  * `target/` d'un crate outil : IGNORE.
  * L'exception assumee est `tools/python-wasm/`, ignore explicitement.

# Ce qui est verifie ici

Pour chaque crate qu'un script de construction compile par `--manifest-path` :

1. Son `Cargo.lock` est suivi par Git, ou le crate figure parmi les exceptions
   declarees dans `.gitignore`.
2. Son `target/` est ignore, pour qu'une compilation lancee sans
   `--target-dir` ne laisse pas non plus un arbre sale.
"""

import re
import subprocess
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]

# Les scripts qui compilent un crate annexe par --manifest-path.
SCRIPTS = (
    "tools/reference/build-trigkey-stage2-usb.ps1",
)


def suivis():
    """Chemins suivis par Git, normalises en separateurs POSIX."""
    sortie = subprocess.run(
        ["git", "-C", str(RACINE), "ls-files"],
        capture_output=True, text=True, check=True,
    ).stdout
    return {ligne.replace("\\", "/") for ligne in sortie.splitlines() if ligne.strip()}


def ignore(chemin):
    """Git ignore-t-il ce chemin ?"""
    return subprocess.run(
        ["git", "-C", str(RACINE), "check-ignore", "-q", chemin],
        capture_output=True,
    ).returncode == 0


def manifestes():
    """Les Cargo.toml designes par un --manifest-path dans les scripts."""
    trouves = set()
    for rel in SCRIPTS:
        chemin = RACINE / rel
        if not chemin.exists():
            continue
        texte = chemin.read_text(encoding="utf-8")
        # $Nom = Join-Path $RepoRoot "tools\...\Cargo.toml"
        for m in re.finditer(r'Join-Path \$RepoRoot "([^"]*Cargo\.toml)"', texte):
            trouves.add(m.group(1).replace("\\", "/"))
    return sorted(trouves)


def main():
    fautes = []
    connus = suivis()
    liste = manifestes()

    if not liste:
        fautes.append(
            "aucun crate annexe trouve dans %s. Soit la construction du shim "
            "UEFI a disparu, soit elle a change de forme et cette garde ne la "
            "voit plus -- dans les deux cas elle ne protege plus rien."
            % ", ".join(SCRIPTS)
        )

    for manifeste in liste:
        dossier = manifeste.rsplit("/", 1)[0]
        lock = dossier + "/Cargo.lock"
        cible = dossier + "/target/"

        if lock not in connus and not ignore(lock):
            fautes.append(
                "%s n'est ni suivi par Git ni ignore. cargo le recree a chaque "
                "compilation, et IMAGE-TRIGKEY.ps1 refuse alors de construire "
                "-- la construction precedente bloque la suivante. Son frere "
                "tools/reference/uefi-image-builder/Cargo.lock est suivi ; "
                "celui-ci doit l'etre aussi, ou etre ignore explicitement."
                % lock
            )

        if not ignore(cible):
            fautes.append(
                "%s n'est pas ignore. Une compilation lancee sans "
                "--target-dir y deposerait des centaines de fichiers, et "
                "l'arbre redeviendrait sale." % cible
            )

    if fautes:
        print("arbre propre apres construction : %d probleme(s)" % len(fautes))
        for faute in fautes:
            print("\n  - %s" % faute)
        return 1

    print(
        "arbre propre apres construction : %d crate(s) annexe(s), lock suivi "
        "et target ignore." % len(liste)
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
