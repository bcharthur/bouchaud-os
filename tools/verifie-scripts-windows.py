#!/usr/bin/env python3
"""Les scripts PowerShell doivent etre en ASCII strict. Voici pourquoi.

    ./tools/verifie-scripts-windows.py

## Le defaut que cette verification existe pour empecher

Windows PowerShell 5.1 — celui qu'un poste Windows a par defaut, sans rien
installer — lit un `.ps1` **sans BOM** avec la page de codes ANSI de la machine,
c'est-a-dire Windows-1252 en Europe de l'Ouest. Un fichier ecrit en UTF-8 y est
donc mal decode.

D'ordinaire, mal decoder un commentaire est sans consequence : le texte est
illisible, le code tourne. Pas ici. Le tiret cadratin est le contre-exemple :

    "-"  en UTF-8  = E2 80 94
    E2 80 94  lu en Windows-1252  =  a-chapeau, euro, GUILLEMET FERMANT

Le troisieme octet, `0x94`, est U+201D — un guillemet double fermant. Et le
lecteur de PowerShell **accepte les guillemets typographiques comme delimiteurs
de chaine**. Un tiret cadratin dans un commentaire ouvre donc une chaine, qui
avale tout jusqu'au tiret suivant : accolades, parentheses et guillemets
compris.

Ce qui casse ne depend alors plus du nombre de tirets mais de **ce qui se trouve
entre eux**. Deux tirets separes par du texte de commentaire ne font rien de
visible ; deux tirets separes par un corps de fonction emportent ses accolades,
et le fichier entier cesse d'etre analysable. C'est exactement ce qui est
arrive : `tools/etat-source.ps1` fonctionnait, `tools/userland.ps1` rendait neuf
erreurs de syntaxe dont la premiere designait une ligne parfaitement correcte.

Un defaut qui depend de la page de codes du lecteur ne se voit sur aucune
machine de developpement Linux, ou tout est UTF-8. Il ne se voit que chez
l'utilisateur.

## Pourquoi l'ASCII plutot qu'un BOM

Un BOM UTF-8 ferait lire correctement le fichier par PowerShell 5.1, et c'est la
solution habituelle. Elle demande cependant que chaque outil qui touche au
fichier le preserve — editeur, `git`, script de generation — et un BOM perdu ne
se signale pas davantage que le probleme qu'il corrige.

L'ASCII, lui, se lit pareil dans **toutes** les pages de codes : il n'y a plus
de decodage a reussir. Les scripts PowerShell du depot etaient d'ailleurs deja
en ASCII avant qu'on y ajoute des accents ; cette verification ne fait que rendre
la regle explicite et verifiable.
"""

import os
import subprocess
import sys
import unicodedata

RACINE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def scripts():
    """Les `.ps1` **suivis par git**, chemins relatifs, tries.

    Suivis par git, et non tous ceux du disque : les arbres de compilation de
    CPython et de Qt en apportent une douzaine qui ne sont pas les notres. Les
    verifier reviendrait a faire echouer la barriere sur le style d'un tiers,
    et a dire quelque chose de faux — nous ne gouvernons que ce que nous
    livrons.
    """
    sortie = subprocess.run(["git", "-C", RACINE, "ls-files", "*.ps1"],
                            capture_output=True, text=True, check=True).stdout
    return sorted(ligne for ligne in sortie.splitlines() if ligne.strip())


def fautes(chemin):
    """Les caracteres non-ASCII d'un fichier, avec leur ligne et leur nom."""
    with open(os.path.join(RACINE, chemin), "rb") as fichier:
        octets = fichier.read()
    if octets.startswith(b"\xef\xbb\xbf"):
        return [(1, "﻿", "MARQUE D'ORDRE DES OCTETS (BOM)")]
    try:
        texte = octets.decode("utf-8")
    except UnicodeDecodeError as e:
        return [(0, "?", "fichier illisible en UTF-8 : %s" % e)]
    trouvees = []
    for numero, ligne in enumerate(texte.splitlines(), start=1):
        for caractere in ligne:
            if ord(caractere) > 127:
                try:
                    nom = unicodedata.name(caractere)
                except ValueError:
                    nom = "U+%04X" % ord(caractere)
                trouvees.append((numero, caractere, nom))
    return trouvees


def principal():
    total = 0
    fichiers = scripts()
    if not fichiers:
        print("aucun script PowerShell trouve — la verification ne prouve rien")
        return 1
    for chemin in fichiers:
        trouvees = fautes(chemin)
        if not trouvees:
            print("  ascii  %s" % chemin)
            continue
        total += len(trouvees)
        print("  ECHEC  %s" % chemin)
        for numero, caractere, nom in trouvees[:10]:
            print("           ligne %-4d %-3r %s" % (numero, caractere, nom))
        if len(trouvees) > 10:
            print("           ... et %d autre(s)" % (len(trouvees) - 10))

    print("")
    if total:
        print("%d caractere(s) non-ASCII dans les scripts PowerShell." % total)
        print("Windows PowerShell 5.1 les lira en Windows-1252, et certains y")
        print("deviennent des guillemets typographiques — que son analyseur")
        print("accepte comme delimiteurs de chaine. Voir l'en-tete de ce")
        print("fichier pour le mecanisme complet.")
        return 1
    print("%d script(s) PowerShell, tous en ASCII strict." % len(fichiers))
    return 0


if __name__ == "__main__":
    sys.exit(principal())
