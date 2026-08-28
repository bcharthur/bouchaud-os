#!/usr/bin/env python3
"""L'ecriture incrementale de `/persist` ne peut pas sauter une ecriture reelle.

# Ce que ce garde-fou protege

`synchronise` ne reecrit plus la zone entiere a chaque `fsync` : il garde
l'empreinte de ce que le dernier `sync` REUSSI a laisse sur le disque, et saute
les fichiers dont le chemin, le secteur, la longueur et le sceau coincident.

Cette optimisation repose sur UNE regle : l'empreinte doit etre oubliee des
qu'une ecriture echoue. Sinon le `sync` suivant croirait sur le disque des
octets qui n'y sont jamais arrives, et les sauterait -- une perte de donnees
silencieuse, que rien dans le journal ne signalerait.

Le compilateur ne peut pas la verifier : `return -1` est un chemin ordinaire.
Ce script le fait, en exigeant que chaque sortie en erreur de `synchronise`
soit precedee de `oublie_le_disque()`.

# Et la deuxieme regle

L'empreinte vit dans un `static mut`. Elle n'est sure que parce que `fsync`,
`fdatasync` et `sync` s'executent sous le gros verrou du noyau. Les liberer
sans reprendre ce probleme les ferait courir en parallele sur quatre coeurs.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
PERSISTANCE = RACINE / "src" / "fs" / "persistance.rs"
BKL = RACINE / "src" / "compat" / "linux" / "bkl.rs"

# Combien de lignes en arriere on accepte de chercher l'oubli. Un `return -1`
# est toujours precede de son `log_fmt`, qui tient sur quelques lignes.
PORTEE = 12


def corps(source: str, signature: str) -> tuple[int, list[str]]:
    """Lignes du corps de la fonction, et le numero de sa premiere ligne."""
    lignes = source.splitlines()
    for numero, ligne in enumerate(lignes):
        if ligne.startswith(signature):
            profondeur = 0
            corps_lignes = []
            for suite in lignes[numero:]:
                profondeur += suite.count("{") - suite.count("}")
                corps_lignes.append(suite)
                if profondeur == 0 and corps_lignes[0].count("{"):
                    break
            return numero + 1, corps_lignes
    raise SystemExit(f"introuvable : {signature}")


def verifie_oublis() -> list[str]:
    source = PERSISTANCE.read_text(encoding="utf-8")
    premiere, lignes = corps(source, "pub fn synchronise()")
    fautes = []
    for decalage, ligne in enumerate(lignes):
        if ligne.strip() != "return -1;":
            continue
        avant = lignes[max(0, decalage - PORTEE) : decalage]
        if not any("oublie_le_disque()" in l for l in avant):
            fautes.append(
                f"  persistance.rs:{premiere + decalage}  `return -1` sans "
                f"`oublie_le_disque()` : l'empreinte survivrait a un echec "
                f"d'ecriture, et le sync suivant sauterait des secteurs qui "
                f"n'ont jamais ete ecrits"
            )
    if "*disque = nouveau;" not in source:
        fautes.append(
            "  persistance.rs  l'empreinte n'est adoptee nulle part : "
            "l'optimisation ne peut pas fonctionner"
        )
    # Elle ne doit etre adoptee qu'a UN endroit, apres l'en-tete.
    if source.count("*disque = nouveau;") != 1:
        fautes.append(
            "  persistance.rs  l'empreinte est adoptee plusieurs fois ; "
            "elle ne doit l'etre qu'apres un sync completement reussi"
        )
    return fautes


def verifie_verrou() -> list[str]:
    source = BKL.read_text(encoding="utf-8")
    fautes = []
    for appel in ("FSYNC", "FDATASYNC", "SYNC"):
        if re.search(rf"\(nr::{appel},", source):
            fautes.append(
                f"  bkl.rs  `{appel}` figure dans SANS_BKL : l'empreinte de "
                f"`/persist` est un `static mut`, elle n'est sure que sous le "
                f"gros verrou"
            )
    return fautes


def main() -> int:
    fautes = verifie_oublis() + verifie_verrou()
    if fautes:
        print("ecriture incrementale de /persist : regle violee")
        print("\n".join(fautes))
        return 1
    print(
        "ok  src/fs/persistance.rs : tout echec oublie l'empreinte, "
        "et sync reste sous le gros verrou"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
