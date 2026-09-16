#!/usr/bin/env python3
"""Garde-fou : ne pas reprendre le verrou RAMFS qu'on tient deja.

# La panique, reproduite le 16 septembre 2026

    *** KERNEL PANIC *** cpu=0
    panicked at src/fs/ramfs.rs:198:8:
    LOCKDEP inversion cpu=0 held_rank=50 acquiring=vfs(50)

`montage::monte` tenait `let mut systeme = fs();` pour tout son corps et
appelait `depose`, qui reprenait `fs()`. Meme verrou, meme rang, meme CPU :
lockdep l'a arrete avant l'interblocage.

# Pourquoi elle a mis si longtemps a sortir

Cette branche ne s'atteint que si la zone de persistance existe ET porte des
entrees. `debut()` la refuse sur un volume de donnees trop petit, et tous les
demarrages QEMU tournaient jusqu'ici sans second disque : `persistance: disque
trop petit, zone absente`, et le code fautif n'etait jamais execute. Le
premier demarrage avec l'image Ladybird -- 1351 mebioctets de donnees --
l'a trouve immediatement.

La meme faute avait DEJA ete corrigee dans ce module, pour `rassemble` et
`collecte` : la note en tete de `collecte.rs` la raconte. Elle avait ete
corrigee la, et pas ici.

# Ce qui est verifie

Dans `src/fs/persistance/`, aucune fonction qui TIENT le garde RAMFS
n'appelle une fonction du meme module qui le REPREND.

L'idiome du module est deja ecrit : une fonction appelee sous le garde
emprunte le `FileSystem` (`collecte_sous_garde`, `depose_sous_garde`). Un
appelant qui ne tient pas le verrou ne compile alors pas, et c'est le
compilateur qui porte l'invariant plutot qu'une relecture.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
MODULE = RACINE / "src/fs/persistance"

# `fn nom(` en debut de ligne, avec ou sans pub/unsafe, hors indentation de bloc.
ENTETE = re.compile(r"^(?:pub(?:\([^)]*\))?\s+)?(?:unsafe\s+)?fn\s+([a-z0-9_]+)\s*[(<]", re.M)


def sans_commentaires(source):
    return "\n".join(
        ligne for ligne in source.splitlines()
        if not ligne.lstrip().startswith("//")
    )


def corps_des_fonctions(source):
    """Rend {nom: corps} pour les fonctions de premier niveau du fichier."""
    fonctions = {}
    for m in ENTETE.finditer(source):
        nom = m.group(1)
        ouvre = source.find("{", m.end() - 1)
        if ouvre < 0:
            continue
        profondeur = 0
        for j in range(ouvre, len(source)):
            if source[j] == "{":
                profondeur += 1
            elif source[j] == "}":
                profondeur -= 1
                if profondeur == 0:
                    fonctions[nom] = source[ouvre:j + 1]
                    break
    return fonctions


def main():
    if not MODULE.is_dir():
        print("  - repertoire absent : %s" % MODULE)
        return 1

    fonctions = {}
    origine = {}
    for chemin in sorted(MODULE.glob("*.rs")):
        source = sans_commentaires(chemin.read_text(encoding="utf-8"))
        for nom, corps in corps_des_fonctions(source).items():
            fonctions[nom] = corps
            origine[nom] = chemin.relative_to(RACINE)

    if not fonctions:
        print("  - aucune fonction lue dans %s ; le garde-fou ne verifie plus rien."
              % MODULE)
        return 1

    # Qui PREND le verrou, qui le TIENT.
    prend = re.compile(r"(?<![a-z_])fs\(\)")
    tient = re.compile(r"=\s*fs\(\)\s*;")
    preneurs = {n for n, c in fonctions.items() if prend.search(c)}
    tenants = {n for n, c in fonctions.items() if tient.search(c)}

    def portee_du_garde(corps, depart):
        """Le texte ou le garde pris en `depart` est encore VIVANT.

        Un garde Rust meurt a la fin du bloc qui le contient. Se contenter de
        « tout ce qui suit dans la fonction » produirait un faux positif des
        qu'un garde est pris dans un bloc interne, et surtout ne distinguerait
        pas un appel AVANT la prise d'un appel APRES : `rassemble_snapshot`
        appelle `rassemble` puis prend le garde, ce qui est correct et a
        pourtant ete signale par une premiere version de ce garde-fou.
        """
        profondeur = 0
        for j in range(depart, len(corps)):
            if corps[j] == "{":
                profondeur += 1
            elif corps[j] == "}":
                profondeur -= 1
                if profondeur < 0:
                    return corps[depart:j]
        return corps[depart:]

    fautes = []
    for nom in sorted(tenants):
        corps = fonctions[nom]
        prises = [m.end() for m in tient.finditer(corps)]
        vivant = "\n".join(portee_du_garde(corps, p) for p in prises)
        for appele in sorted(preneurs):
            if appele == nom:
                continue
            if re.search(r"(?<![a-z_])%s\s*\(" % re.escape(appele), vivant):
                fautes.append(
                    "%s::%s TIENT le garde RAMFS et appelle %s::%s, qui le "
                    "REPREND.\n\n    Meme verrou, meme rang, meme CPU : c'est "
                    "l'inversion que lockdep arrete par une panique noyau "
                    "(`LOCKDEP inversion acquiring=vfs(50)`), et elle ne se "
                    "voit qu'une fois la zone de persistance non vide -- donc "
                    "pas sur un QEMU sans second disque.\n\n    L'idiome du "
                    "module est deja ecrit : faire emprunter le `FileSystem` "
                    "a l'appele, comme `collecte_sous_garde`. Le compilateur "
                    "porte alors l'invariant."
                    % (origine[nom].name, nom, origine[appele].name, appele)
                )

    if fautes:
        print("verrou ramfs : %d probleme(s)" % len(fautes))
        for faute in fautes:
            print("\n  - %s" % faute)
        return 1

    print(
        "verrou ramfs : %d fonction(s) tiennent le garde, %d le prennent, "
        "aucune imbrication" % (len(tenants), len(preneurs))
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
