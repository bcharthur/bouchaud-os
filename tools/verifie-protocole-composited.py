#!/usr/bin/env python3
"""Le protocole de `composited` existe deux fois : il doit dire la meme chose.

Le format du fil vit dans `src/gui/composited.rs` (cote noyau, et cote test
hote) et dans `userland/services/composited/protocole.h` (cote service ring 3).
Rien dans la chaine de construction ne les relie : le premier se compile pour
`x86_64-bouchaud_os`, le second pour la cible ring 3, et ils ne se rencontrent
qu'a l'execution -- ou un desaccord d'un seul octet se manifeste sous la forme
d'une surface qui ne s'ouvre pas, et d'aucun message.

Ce script est le lien manquant. Il ne verifie pas le code : il verifie les
VALEURS sur lesquelles les deux cotes doivent s'accorder, c'est-a-dire
exactement ce qu'un message ajoute ou un champ deplace casse.

Il verifie aussi que les deux protocoles graphiques -- celui du bureau et celui
du compositeur -- ne portent pas le meme nombre magique : un client branche sur
le mauvais service doit echouer tout de suite, pas interpreter des rectangles
au hasard.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
RUST = RACINE / "src" / "gui" / "composited_corps.rs"
RUST_BUREAU = RACINE / "src" / "gui" / "protocole.rs"
C = RACINE / "userland" / "services" / "composited" / "protocole.h"


def entier(texte: str) -> int:
    return int(texte.replace("_", "").replace("u", "").replace("'", ""), 0)


def constantes_rust(source: str) -> dict[str, int]:
    valeurs = {}
    for nom, valeur in re.findall(
        r"pub const (\w+): \w+ = (0x[0-9A-Fa-f_]+|\d[\d_]*);", source
    ):
        valeurs[nom] = entier(valeur)
    for nom, valeur in re.findall(
        r"^\s*(\w+) = (0x[0-9A-Fa-f]+|\d+),", source, re.M
    ):
        valeurs.setdefault(nom, entier(valeur))
    return valeurs


def constantes_c(source: str) -> dict[str, int]:
    valeurs = {}
    for nom, valeur in re.findall(
        r"#define\s+(\w+)\s+(0x[0-9A-Fa-f]+u?|\d+u?)\b", source
    ):
        valeurs[nom] = entier(valeur)
    return valeurs


# Nom Rust -> nom C. Ce tableau EST le contrat : y ajouter une ligne est la
# facon de rendre un nouveau champ obligatoire des deux cotes.
CORRESPONDANCE = {
    "MAGIC": "COMPOSITED_MAGIC",
    "VERSION": "COMPOSITED_VERSION",
    "TAILLE_ENTETE": "COMPOSITED_ENTETE",
    "CHARGE_MAX": "COMPOSITED_CHARGE_MAX",
    "TAMPONS": "COMPOSITED_TAMPONS",
    "SURFACES_MAX": "COMPOSITED_SURFACES_MAX",
    "TAILLE_SURFACE_ACCORDEE": "COMPOSITED_TAILLE_SURFACE_ACCORDEE",
    "TAILLE_TRAME_LIVREE": "COMPOSITED_TAILLE_TRAME_LIVREE",
    "TAILLE_TAMPON_RENDU": "COMPOSITED_TAILLE_TAMPON_RENDU",
    # Genres de message.
    "DemandeSurface": "COMPOSITED_DEMANDE_SURFACE",
    "TrameLivree": "COMPOSITED_TRAME_LIVREE",
    "Detache": "COMPOSITED_DETACHE",
    "SurfaceAccordee": "COMPOSITED_SURFACE_ACCORDEE",
    "TamponRendu": "COMPOSITED_TAMPON_RENDU",
    "Reconfigure": "COMPOSITED_RECONFIGURE",
    "Refus": "COMPOSITED_REFUS",
    # Raisons de refus.
    "PlusDeSurface": "COMPOSITED_REFUS_PLUS_DE_SURFACE",
    "GeometrieInvalide": "COMPOSITED_REFUS_GEOMETRIE",
    "DejaAttache": "COMPOSITED_REFUS_DEJA_ATTACHE",
    "TamponNonPossede": "COMPOSITED_REFUS_TAMPON_NON_POSSEDE",
    "Inconnue": "COMPOSITED_REFUS_INCONNUE",
}


def main() -> int:
    for chemin in (RUST, RUST_BUREAU, C):
        if not chemin.exists():
            print(f"introuvable : {chemin}")
            return 1

    rust = constantes_rust(RUST.read_text(encoding="utf-8"))
    c = constantes_c(C.read_text(encoding="utf-8"))
    bureau = constantes_rust(RUST_BUREAU.read_text(encoding="utf-8"))

    fautes = []
    verifiees = 0
    for nom_rust, nom_c in sorted(CORRESPONDANCE.items()):
        if nom_rust not in rust:
            fautes.append(f"  {nom_rust} absent de composited.rs")
            continue
        if nom_c not in c:
            fautes.append(f"  {nom_c} absent de protocole.h")
            continue
        if rust[nom_rust] != c[nom_c]:
            fautes.append(
                f"  {nom_rust}={rust[nom_rust]} mais {nom_c}={c[nom_c]}"
            )
            continue
        verifiees += 1

    # Les deux protocoles graphiques doivent se distinguer a l'octet pres.
    if rust.get("MAGIC") == bureau.get("MAGIC"):
        fautes.append(
            "  le compositeur et le bureau portent le MEME nombre magique : un "
            "client branche sur le mauvais service interpreterait des "
            "rectangles au hasard au lieu d'echouer"
        )

    # Un genre client et un genre compositeur ne doivent jamais se croiser.
    clients = {rust[n] for n in ("DemandeSurface", "TrameLivree", "Detache") if n in rust}
    reponses = {
        rust[n] for n in ("SurfaceAccordee", "TamponRendu", "Reconfigure", "Refus")
        if n in rust
    }
    if clients & reponses:
        fautes.append(
            "  un genre est a la fois client et reponse : un client pourrait "
            "s'accorder une surface a lui-meme"
        )
    if any(valeur >= 0x100 for valeur in clients):
        fautes.append("  un genre client depasse 0x100 : `du_client()` mentirait")
    if any(valeur < 0x100 for valeur in reponses):
        fautes.append("  une reponse est sous 0x100 : `du_client()` mentirait")

    if fautes:
        print("protocole composited : desaccord")
        print("\n".join(fautes))
        return 1

    print(f"ok  {verifiees} valeurs, noyau et service ring 3 d'accord")
    return 0


if __name__ == "__main__":
    sys.exit(main())
