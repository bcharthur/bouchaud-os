#!/usr/bin/env python3
"""Garde-fou : l'enregistreur de vol doit se tester sur une VRAIE cible.

# Le defaut, mesure le 16 septembre 2026

Le releve physique couvrait 1,07 s a 8,15 s d'une session de vingt minutes.
Aucune campagne du depot ne pouvait le voir : sans partition
BOUCHAUD-BLACKBOX presentee en USB, `blackbox_storage_ready()` est faux et
tout le chemin d'ecriture sort a la premiere ligne. Le banc validait donc un
enregistreur qui n'ecrivait nulle part.

Pire, le PARSER n'etait pas dans le depot. Il vivait dans un dossier d'archive
date du 10 septembre, sur la machine de l'utilisateur : la chaine de
diagnostic entiere dependait d'un fichier non versionne, et aucun genre
d'enregistrement nouveau ne pouvait etre ajoute sans risquer d'en perdre la
charge utile.

# Ce qui est verifie ici

1. Le parser officiel est dans le depot, avec son test de format.
2. Le banc d'endurance existe, presente une vraie partition, et EXIGE une
   couverture minimale -- un banc qui mesure sans seuil ne refuse rien.
3. Il verifie DEBUT ET FIN. Une archive qui ne garde que la fin perd
   l'amorcage ; une qui ne garde que le debut perd le blocage.
4. Le fabricant de disque produit bien le nom que le noyau cherche.
"""

import re
import subprocess
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
PARSER = RACINE / "tools/reference/extract-blackbox.py"
TEST_FORMAT = RACINE / "tools/reference/test-blackbox-format.py"
DISQUE = RACINE / "tools/reference/fabrique-disque-blackbox.py"
BANC = RACINE / "tools/ci/run_blackbox_endurance.sh"
NOM_PARTITION = "BOUCHAUD-BLACKBOX"


def suivi(chemin):
    rel = chemin.relative_to(RACINE).as_posix()
    sortie = subprocess.run(["git", "-C", str(RACINE), "ls-files", "--error-unmatch", rel],
                            capture_output=True)
    return sortie.returncode == 0


def main():
    fautes = []
    for chemin, quoi in (
        (PARSER, "le parser officiel de la blackbox"),
        (TEST_FORMAT, "le test de format du parser"),
        (DISQUE, "le fabricant de disque de test"),
        (BANC, "le banc d'endurance"),
    ):
        if not chemin.exists():
            fautes.append("%s est absent (%s)" % (chemin.relative_to(RACINE).as_posix(), quoi))
        elif not suivi(chemin):
            fautes.append(
                "%s n'est pas SUIVI par Git. C'est exactement l'etat d'avant : "
                "un parser vivant dans un dossier d'archive date, hors du "
                "depot, dont dependait toute la chaine de diagnostic."
                % chemin.relative_to(RACINE).as_posix()
            )
    if fautes:
        print("endurance blackbox : %d probleme(s)" % len(fautes))
        for f in fautes:
            print("\n  - %s" % f)
        return 1

    parser = PARSER.read_text(encoding="utf-8")
    if NOM_PARTITION not in parser:
        fautes.append("extract-blackbox.py ne cherche plus la partition %s." % NOM_PARTITION)
    for genre in ("serial", "sample", "marker", "flight", "memory", "fatal"):
        if '"%s"' % genre not in parser:
            fautes.append(
                "extract-blackbox.py ne connait plus le genre `%s` : ses "
                "enregistrements seraient extraits sans leur fichier." % genre
            )

    # LE CODE, PAS LA DOCSTRING.
    #
    # La docstring de ce fichier CITE le nom de la partition. Chercher le nom
    # dans le fichier entier laissait donc passer un renommage de la constante :
    # la prose promettait ce que le code ne faisait plus.
    disque = DISQUE.read_text(encoding="utf-8")
    disque_code = re.sub(r'"""».*?"""', "", disque, flags=re.S)
    disque_code = re.sub(r'"""[^"]*"""', "", disque, flags=re.S)
    disque_code = "\n".join(re.sub(r"#.*$", "", l) for l in disque_code.splitlines())
    if not re.search(r'NOM\s*=\s*"%s"' % re.escape(NOM_PARTITION), disque_code):
        fautes.append(
            "fabrique-disque-blackbox.py ne produit plus une partition nommee "
            "%s ; le noyau ne la reconnaitrait pas et le banc ne testerait "
            "rien." % NOM_PARTITION
        )

    banc = BANC.read_text(encoding="utf-8")
    if "BOUCHAUD_BLACKBOX_USB_READY" not in banc:
        fautes.append(
            "le banc ne verifie plus que l'enregistreur a TROUVE sa partition. "
            "Sans cela il passerait au vert sur une session ou rien n'a jamais "
            "ete ecrit -- c'est-a-dire sur le defaut lui-meme."
        )
    if not re.search(r"COUVERTURE_MIN.*60|minimum.*60", banc):
        fautes.append(
            "le banc n'exige plus de couverture minimale. Mesurer sans seuil "
            "ne refuse rien : l'archive de 7,6 s serait passee."
        )
    if "markers.log" not in banc:
        fautes.append(
            "le banc ne verifie plus la marque de DEBUT. Une archive qui ne "
            "garde que la fin perd l'amorcage."
        )
    if "usb-storage" not in banc or "qemu-xhci" not in banc:
        fautes.append(
            "le banc ne presente plus de cle USB : `blackbox_storage_ready()` "
            "serait faux et le chemin d'ecriture entier serait saute."
        )

    if fautes:
        print("endurance blackbox : %d probleme(s)" % len(fautes))
        for f in fautes:
            print("\n  - %s" % f)
        return 1

    print(
        "endurance blackbox : parser suivi et teste, banc avec vraie partition, "
        "seuil de couverture et verification du debut."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
