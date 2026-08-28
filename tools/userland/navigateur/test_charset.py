#!/usr/bin/env python3
"""Une page arrive-t-elle dans le bon jeu de caracteres ?

# Le defaut

Le navigateur ne lisait le `charset` que dans l'en-tete HTTP. Or l'en-tete ne le
porte que dans une minorite de cas : le reste du web le declare dans le
document, par `<meta charset>`.

Une page en ISO-8859-1 lue comme de l'UTF-8 rend un `U+FFFD` par octet
accentue, c'est-a-dire un CARRE VIDE a la place de chaque lettre accentuee.
C'est exactement ce qu'on voyait sur les captures d'ecran :
« Avant d'acc<carre>der <carre> Google ».

# La regle

Celle de la norme HTML, du plus fort au plus faible : la marque d'ordre des
octets, puis l'en-tete HTTP, puis le `<meta charset>` du premier kilo-octet,
puis UTF-8.

`decode_charge` est extraite du module plutot qu'importee : `reseau.py` tire
`ssl`, `http.client` et le stockage du navigateur, dont rien ici n'a besoin. Le
code teste reste celui du fichier, a la ligne pres.

Lance par `tools/userland/test-charset.sh`.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent
SOURCE = RACINE / "moteur" / "reseau.py"


def charge_fonction():
    texte = SOURCE.read_text(encoding="utf-8")
    debut = texte.index("_MOTIF_META_CHARSET")
    fin = texte.index("def _charge_http(")
    espace = {"re": re}
    exec(compile(texte[debut:fin], str(SOURCE), "exec"), espace)  # noqa: S102
    return espace["decode_charge"]


CAS = [
    # (description, octets, content-type, texte attendu)
    (
        "BOM UTF-8, sans en-tete",
        "﻿Avant d'acceder a Google".replace("acceder", "accéder")
        .replace(" a ", " à ").encode("utf-8"),
        "",
        "Avant d'accéder à Google",
    ),
    (
        "en-tete UTF-8 explicite",
        "Avant d'accéder à Google".encode("utf-8"),
        "text/html; charset=UTF-8",
        "Avant d'accéder à Google",
    ),
    (
        "en-tete ISO-8859-1",
        "Café".encode("latin-1"),
        "text/html; charset=ISO-8859-1",
        "Café",
    ),
    (
        "en-tete avec guillemets",
        "Café".encode("latin-1"),
        'text/html; charset="iso-8859-1"',
        "Café",
    ),
    (
        "meta charset, en-tete muet",
        b"<html><head><meta charset=\"iso-8859-1\"><title>Caf\xe9</title>",
        "text/html",
        "Café",
    ),
    (
        "meta http-equiv, en-tete absent",
        b"<meta http-equiv='content-type' "
        b"content='text/html; charset=windows-1252'>Caf\xe9",
        "",
        "Café",
    ),
    (
        "l'en-tete prime sur le meta",
        "Café".encode("utf-8") + b"<meta charset='iso-8859-1'>",
        "text/html; charset=utf-8",
        "Café",
    ),
    (
        "le BOM prime sur l'en-tete",
        b"\xef\xbb\xbf" + "Café".encode("utf-8"),
        "text/html; charset=iso-8859-1",
        "Café",
    ),
    (
        "defaut UTF-8 quand rien n'est dit",
        "Café".encode("utf-8"),
        "text/html",
        "Café",
    ),
    (
        "jeu inconnu : on retombe sur UTF-8 sans exception",
        "Café".encode("utf-8"),
        "text/html; charset=x-nimportequoi",
        "Café",
    ),
    (
        "UTF-16 petit-boutiste",
        b"\xff\xfe" + "Café".encode("utf-16-le"),
        "",
        "Café",
    ),
    (
        "UTF-16 gros-boutiste",
        b"\xfe\xff" + "Café".encode("utf-16-be"),
        "",
        "Café",
    ),
]


def main():
    decode_charge = charge_fonction()
    echecs = []

    for description, octets, content_type, attendu in CAS:
        obtenu = decode_charge(octets, content_type)
        if attendu not in obtenu:
            echecs.append(
                f"  {description}\n"
                f"    attendu : {attendu!r}\n"
                f"    obtenu  : {obtenu!r}"
            )

    # La propriete qui compte vraiment : AUCUN carre de remplacement pour une
    # page correctement declaree. C'est le symptome observe, et il ne doit
    # revenir d'aucune des routes ci-dessus.
    for description, octets, content_type, _ in CAS:
        obtenu = decode_charge(octets, content_type)
        if "�" in obtenu:
            echecs.append(
                f"  {description} : un caractere de remplacement U+FFFD "
                f"subsiste — c'est le carre vide qu'on voit a l'ecran"
            )

    # Et un cas qui doit rester tolerant : des octets reellement invalides ne
    # doivent pas faire echouer le chargement de toute la page.
    abime = decode_charge(b"Caf\xff\xfe\x00 suite", "text/html; charset=utf-8")
    if "suite" not in abime:
        echecs.append("  des octets invalides ne doivent pas perdre le reste "
                      "de la page")

    # Une declaration tardive ne doit PAS etre prise en compte : la norme borne
    # la recherche au premier kilo-octet, et un analyseur qui accepterait plus
    # loin devrait redecoder la page en cours de route.
    tardif = decode_charge(b"<!--" + b"x" * 1200 + b"--><meta charset='iso-8859-1'>"
                           + b"Caf\xe9", "text/html")
    if "�" not in tardif:
        echecs.append("  une declaration au-dela du premier kilo-octet ne doit "
                      "pas etre suivie")

    if echecs:
        print("jeu de caracteres : regle violee")
        print("\n".join(echecs))
        return 1

    print(f"ok  moteur/reseau.py : {len(CAS)} cas de jeu de caracteres, "
          f"BOM, en-tete, meta et defaut")
    return 0


if __name__ == "__main__":
    sys.exit(main())
