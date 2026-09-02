#!/usr/bin/env python3
"""Aucun mot de passe ne doit revenir en clair, ni aucun compte par defaut.

# Ce que ce garde-fou protege

Les mots de passe etaient stockes EN CLAIR dans la table des comptes, compares
par `==`, et les deux comptes du systeme naissaient avec leur propre nom comme
mot de passe -- `root:root`, `guest:guest`.

Chacun de ces trois defauts se reintroduit d'une ligne, et aucun ne fait
echouer quoi que ce soit : le systeme continue de fonctionner exactement pareil.
C'est precisement pour cela qu'ils ont survecu si longtemps.

# Les quatre regles

  1. La table des comptes ne porte pas de champ de mot de passe en clair.
  2. L'authentification passe par l'empreinte, jamais par une comparaison
     directe de chaines.
  3. Aucun compte n'est cree avec un mot de passe fige dans le code.
  4. La comparaison d'empreintes accumule les differences au lieu de sortir a
     la premiere -- sinon le temps de reponse revele le secret octet par octet.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
UTILISATEURS = RACINE / "src" / "users" / "mod.rs"
EMPREINTE = RACINE / "src" / "users" / "motdepasse.rs"


def main() -> int:
    fautes = []
    for chemin in (UTILISATEURS, EMPREINTE):
        if not chemin.exists():
            print(f"introuvable : {chemin}")
            return 1

    source = UTILISATEURS.read_text(encoding="utf-8")
    code = re.sub(r"//[^\n]*", "", source)
    empreinte = re.sub(r"//[^\n]*", "", EMPREINTE.read_text(encoding="utf-8"))

    # --- 1. plus de mot de passe en clair dans la table --------------------
    if re.search(r"\bpass\s*:\s*\[u8;", code) or "pass_len" in code:
        fautes.append(
            "  users/mod.rs  la table des comptes reporte un mot de passe en "
            "clair : toute lecture de la memoire noyau -- image de panique, "
            "pilote qui deborde -- les rendrait tous"
        )
    if "empreinte: Empreinte" not in code:
        fautes.append(
            "  users/mod.rs  la table ne porte plus d'empreinte : "
            "l'authentification ne peut plus etre sure"
        )

    # --- 2. l'authentification passe par l'empreinte -----------------------
    authentification = re.search(
        r"pub fn authenticate\((?:[^)]*)\)[^{]*\{(.*?)\n\}", code, re.S)
    if not authentification:
        fautes.append("  users/mod.rs  `authenticate` a disparu")
    elif "mot_de_passe_correct" not in authentification.group(1):
        fautes.append(
            "  users/mod.rs  `authenticate` ne passe plus par l'empreinte"
        )

    # --- 3. aucun mot de passe fige dans le code ---------------------------
    #
    # Un identifiant par defaut connu n'est pas une commodite : il est le meme
    # sur toutes les installations, presque jamais change, et suffit a prendre
    # l'uid 0.
    for appel in re.finditer(r'create\(\s*"([^"]+)"\s*,\s*"([^"]*)"', code):
        fautes.append(
            f"  users/mod.rs  le compte `{appel.group(1)}` est cree avec le mot "
            f"de passe fige « {appel.group(2)} » : c'est le meme sur toutes les "
            f"installations, et il suffit a lui seul pour entrer"
        )
    initialisation = re.search(r"pub fn init\(\)[^{]*\{(.*?)\n\}", code, re.S)
    if initialisation and "create_verrouille" not in initialisation.group(1):
        fautes.append(
            "  users/mod.rs  `init` ne cree plus les comptes verrouilles : "
            "ils naitraient avec un mot de passe utilisable"
        )

    # --- 4. la comparaison ne sort pas tot ---------------------------------
    comparaison = re.search(
        r"pub fn egal_temps_constant\((?:[^)]*)\)[^{]*\{(.*?)\n\}", empreinte, re.S)
    if not comparaison:
        fautes.append("  motdepasse.rs  `egal_temps_constant` a disparu")
    else:
        corps = comparaison.group(1)
        if "|=" not in corps or "^" not in corps:
            fautes.append(
                "  motdepasse.rs  la comparaison n'accumule plus les "
                "differences : sortir a la premiere revele, par le temps de "
                "reponse, combien d'octets sont justes"
            )
        if "return true" in corps or re.search(r"if\s+a\[[^\]]+\]\s*!=", corps):
            fautes.append(
                "  motdepasse.rs  la comparaison sort en avance dans la boucle"
            )

    # --- 5. l'etat verrouille reste distinct du mot de passe vide ----------
    if "est_verrouille" not in empreinte or "defini" not in empreinte:
        fautes.append(
            "  motdepasse.rs  l'etat « aucun mot de passe » a disparu : le "
            "confondre avec « mot de passe vide » ferait accepter la chaine "
            "vide sur tout compte neuf"
        )

    if fautes:
        print("mots de passe : regle violee")
        print("\n".join(fautes))
        return 1

    print(
        "ok  src/users : aucun mot de passe en clair, aucun compte par defaut, "
        "comparaison a temps constant"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
