#!/usr/bin/env python3
"""Garde-fou : le compositeur ring 3 est EXECUTE, pas seulement compile.

`userland/services/composited/composited-slice.c` porte le tranchant vertical du
compositeur en espace utilisateur : demande de surface, transfert de capacite
attenuee, propriete des tampons, reveil par waitset, retour du tampon libere.

Il etait COMPILE par CI Fast et jamais EXECUTE -- un commentaire du workflow
affirmait meme le contraire. Compiler un contrat ne prouve que sa syntaxe : les
appels natifs, l'attenuation des droits et la propriete des tampons ne se
verifient qu'en ring 3, sur un vrai noyau.

Ce garde-fou verifie les quatre maillons de l'execution reelle :

  1. le harnais QEMU CONSTRUIT le service ;
  2. il le PLACE dans l'image, par une option que le fabricant d'image connait ;
  3. il EXIGE le verdict `COMPOSITED_SLICE_OK` ;
  4. les marqueurs exiges existent MOT POUR MOT dans le service.

Le quatrieme est le moins evident et le plus utile : un marqueur qui ne
correspond a rien fait echouer la CI, mais un marqueur RENOMME du cote du
service et pas du cote du harnais transformerait la verification en formalite
qu'aucune regression ne peut plus casser.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
SERVICE = RACINE / "userland/services/composited/composited-slice.c"
HARNAIS = RACINE / "tools/ci/run_native_ipc_probe.sh"
IMAGE = RACINE / "tools/native/make_native_ipc_probe_image.py"


def main() -> int:
    fautes = []
    for chemin in (SERVICE, HARNAIS, IMAGE):
        if not chemin.exists():
            print("  - fichier absent : %s" % chemin.relative_to(RACINE).as_posix())
            return 1

    service = SERVICE.read_text(encoding="utf-8")
    harnais = HARNAIS.read_text(encoding="utf-8")
    image = IMAGE.read_text(encoding="utf-8")

    if "userland/services/composited" not in harnais:
        fautes.append(
            "run_native_ipc_probe.sh : le service n'est plus construit par le harnais "
            "QEMU ; il redevient un binaire que personne n'execute."
        )
    if "--composited" not in harnais:
        fautes.append(
            "run_native_ipc_probe.sh : le service n'est plus place dans l'image ; "
            "l'autorun ne peut plus le lancer."
        )
    if "--composited" not in image:
        fautes.append(
            "make_native_ipc_probe_image.py : l'option `--composited` a disparu ; "
            "le harnais la passe dans le vide."
        )
    if "/bin/composited-slice" not in image:
        fautes.append(
            "make_native_ipc_probe_image.py : l'autorun ne lance plus "
            "`/bin/composited-slice`."
        )
    if "COMPOSITED_SLICE_OK" not in harnais:
        fautes.append(
            "run_native_ipc_probe.sh : le verdict du service n'est plus exige ; "
            "un tranchant qui echoue passerait inapercu."
        )
    if "COMPOSITED_SLICE_FAIL" not in harnais:
        fautes.append(
            "run_native_ipc_probe.sh : l'echec du service n'est plus detecte tot ; "
            "la CI attendrait l'echeance au lieu de dire pourquoi."
        )

    # Les marqueurs exiges doivent exister MOT POUR MOT dans le service.
    noms = set(re.findall(r'exige\([^;]*?"((?:[^"\\]|\\.)*)"', service, re.S))
    exiges = [
        ligne.strip().strip("'").split("[COMPOSITED] ok   ", 1)[1]
        for ligne in harnais.splitlines()
        if "[COMPOSITED] ok   " in ligne
    ]
    if not exiges:
        fautes.append(
            "run_native_ipc_probe.sh : plus aucun marqueur de succes du compositeur "
            "n'est exige ; `COMPOSITED_SLICE_OK` seul ne dit pas ce qui a ete prouve."
        )
    for attendu in exiges:
        if attendu not in noms:
            fautes.append(
                "run_native_ipc_probe.sh exige un marqueur que le service n'imprime "
                "pas : %r. Un marqueur renomme d'un seul cote ferait echouer la CI ; "
                "renomme des DEUX cotes." % attendu
            )

    if fautes:
        print("execution composited : %d probleme(s)\n" % len(fautes))
        for faute in fautes:
            print("  - %s\n" % faute)
        return 1

    print(
        "composited : construit, place dans l'image, execute en ring 3, "
        "%d marqueur(s) exige(s) et presents" % len(exiges)
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
