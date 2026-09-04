#!/usr/bin/env python3
"""Le systeme de fichiers ne doit pas reprendre de dependance directe a ATA.

# Ce que ce garde-fou protege

`drivers::api::block` definissait deja un trait `BlockDevice`. Il etait correct,
et il etait inutilisable pour ajouter NVMe : ses fonctions libres prenaient un
`ata::Drive`, et le systeme de fichiers appelait `ata::read(Drive::Slave, ...)`
directement. Le trait existait ; personne ne passait par lui, et l'ajout d'un
second pilote aurait demande de reecrire les appelants -- exactement ce que le
trait devait eviter.

`drivers::bloc` remplace la nappe par un VOLUME. La migration se fait appelant
par appelant : chacun demande de traiter ses chemins d'erreur, et les faire tous
d'un coup serait un changement qu'on ne saurait pas bissecter.

# La regle

Le nombre d'appels directs a ATA dans `src/fs/` ne doit pas AUGMENTER. C'est un
budget, pas une interdiction : interdire d'un coup rendrait le depot rouge sans
rien migrer, et un controle qu'on ne peut pas rendre vert n'est pas une
barriere.

Chaque appelant migre se retire du budget, en le baissant ici. Le jour ou il
vaut zero, la regle devient l'interdiction qu'elle veut etre.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
FS = RACINE / "src" / "fs"

# Budget par fichier. Baisser une valeur est la facon de constater une
# migration ; l'augmenter demande de dire pourquoi, ici, en toutes lettres.
BUDGET = {
    "src/fs/persistance/io.rs": 3,
    "src/fs/persistance/montage.rs": 3,
    # `format.rs` porte encore une lecture : celle des deux superblocs. La
    # capacite du volume, elle, passe deja par `drivers::bloc`.
    "src/fs/persistance/format.rs": 1,
    "src/fs/tar.rs": 4,
}

APPEL = re.compile(r"\bata::(read|write|capacities|present|probe)\b")


def main() -> int:
    if not FS.exists():
        print(f"introuvable : {FS}")
        return 1

    courant: dict[str, int] = {}
    for chemin in sorted(FS.rglob("*.rs")):
        relatif = chemin.relative_to(RACINE).as_posix()
        compte = 0
        for ligne in chemin.read_text(encoding="utf-8", errors="replace").split("\n"):
            if ligne.lstrip().startswith("//"):
                continue
            compte += len(APPEL.findall(ligne))
        if compte:
            courant[relatif] = compte

    fautes, gains = [], []
    for fichier in sorted(set(BUDGET) | set(courant)):
        budget = BUDGET.get(fichier, 0)
        mesure = courant.get(fichier, 0)
        if mesure > budget:
            fautes.append(
                f"  {fichier} : {mesure} appels directs a ATA > {budget} (budget). "
                f"Passer par `drivers::bloc` plutot que de reprendre la nappe."
            )
        elif mesure < budget:
            gains.append(f"  {fichier} : {mesure} < {budget}, budget a resserrer")

    total = sum(courant.values())
    if fautes:
        print("couche bloc : regle violee")
        print("\n".join(fautes))
        return 1

    if gains:
        print("couche bloc : progres non adopte")
        print("\n".join(gains))
    print(
        f"ok  {total} appel(s) direct(s) a ATA dans src/fs, tous sous budget ; "
        f"la capacite du volume passe deja par `drivers::bloc`"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
