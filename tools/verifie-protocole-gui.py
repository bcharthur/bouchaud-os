#!/usr/bin/env python3
"""Verifie que les deux implementations du protocole GUI disent la meme chose.

Le format de fil existe deux fois : dans `src/gui/protocole.rs`, cote noyau, et
dans `tools/userland/navigateur/hote.cpp`, cote client Qt. Rien dans la chaine de
construction ne relie les deux — le noyau se compile pour `x86_64-bouchaud_os`,
l'hote pour la cible userland, et ils ne se rencontrent qu'a l'execution, dans
QEMU, ou un desaccord se manifeste sous la forme d'une fenetre qui ne s'ouvre
pas et d'aucun message.

Ce script est le lien manquant. Il ne verifie pas le code : il verifie les
**valeurs sur lesquelles les deux cotes doivent s'accorder**, c'est-a-dire
exactement ce qu'un ajout de message ou un renumerotage casse.

  - le nombre magique et la version ;
  - la taille de l'en-tete et le plafond de charge utile ;
  - la valeur numerique de chaque genre de message ;
  - les codes de touche du protocole.

Code de retour : 0 si les deux sont d'accord, 1 sinon.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
RUST = RACINE / "src" / "gui" / "protocole.rs"
CPP = RACINE / "tools" / "userland" / "navigateur" / "hote.cpp"


def entier(texte):
    """Lit un entier decimal ou hexadecimal, avec ou sans separateurs `_`."""
    return int(texte.replace("_", "").replace("'", ""), 0)


def constantes_rust(source):
    valeurs = {}
    for nom, valeur in re.findall(
        r"pub const (\w+): \w+ = (0x[0-9A-Fa-f_]+|\d+);", source
    ):
        valeurs[nom] = entier(valeur)

    genres = {}
    bloc = re.search(r"pub enum Genre \{(.*?)\n\}", source, re.S)
    if not bloc:
        raise SystemExit("protocole.rs : enum Genre introuvable")
    for nom, valeur in re.findall(r"(\w+) = (0x[0-9A-Fa-f]+|\d+),", bloc.group(1)):
        genres[nom] = entier(valeur)
    return valeurs, genres


def constantes_cpp(source):
    bloc = re.search(r"namespace protocole \{(.*?)\n\} // namespace protocole", source, re.S)
    if not bloc:
        raise SystemExit("hote.cpp : namespace protocole introuvable")
    corps = bloc.group(1)

    valeurs = {}
    for nom, valeur in re.findall(
        r"constexpr \w+ (\w+) = (0x[0-9A-Fa-f']+|\d+);", corps
    ):
        valeurs[nom] = entier(valeur)

    genres = {}
    enum_genre = re.search(r"enum Genre : uint16_t \{(.*?)\n\};", corps, re.S)
    if not enum_genre:
        raise SystemExit("hote.cpp : enum Genre introuvable")
    for nom, valeur in re.findall(r"(\w+) = (0x[0-9A-Fa-f]+|\d+),", enum_genre.group(1)):
        genres[nom] = entier(valeur)

    touches = {}
    enum_touche = re.search(r"enum CodeTouche : uint32_t \{(.*?)\n\};", corps, re.S)
    if enum_touche:
        for nom, valeur in re.findall(r"(\w+) = (\d+),", enum_touche.group(1)):
            touches[nom] = entier(valeur)
    return valeurs, genres, touches


def codes_touche_rust():
    """Les codes de touche vivent dans le gestionnaire de fenetres : c'est lui
    qui les produit, et il n'y a aucune raison que le protocole les connaisse."""
    source = (RACINE / "src" / "gui" / "window_manager.rs").read_text(encoding="utf-8")
    bloc = re.search(r"mod touche \{(.*?)\n\}", source, re.S)
    if not bloc:
        raise SystemExit("window_manager.rs : module `touche` introuvable")
    return {
        nom: entier(valeur)
        for nom, valeur in re.findall(r"pub const (\w+): u32 = (\d+);", bloc.group(1))
    }


def compare(quoi, gauche, droite, correspondances, echecs):
    for nom_rust, nom_cpp in correspondances.items():
        a = gauche.get(nom_rust)
        b = droite.get(nom_cpp)
        if a is None:
            echecs.append(f"{quoi} : {nom_rust} absent cote Rust")
        elif b is None:
            echecs.append(f"{quoi} : {nom_cpp} absent cote C++")
        elif a != b:
            echecs.append(f"{quoi} : {nom_rust}={a} mais {nom_cpp}={b}")
        else:
            print(f"  ok     {quoi} {nom_rust} = {a}")


def main():
    valeurs_rust, genres_rust = constantes_rust(RUST.read_text(encoding="utf-8"))
    valeurs_cpp, genres_cpp, touches_cpp = constantes_cpp(CPP.read_text(encoding="utf-8"))
    touches_rust = codes_touche_rust()

    echecs = []

    compare(
        "constante",
        valeurs_rust,
        valeurs_cpp,
        {
            "MAGIC": "MAGIC",
            "PROTOCOL_VERSION": "VERSION",
            "TAILLE_ENTETE": "TAILLE_ENTETE",
            "CHARGE_MAX": "CHARGE_MAX",
            "FENETRE_PRINCIPALE": "FENETRE",
        },
        echecs,
    )

    # Chaque genre declare d'un cote doit exister de l'autre avec la meme
    # valeur : c'est la moitie du protocole qu'un renumerotage casse en silence.
    for nom in sorted(set(genres_rust) | set(genres_cpp)):
        compare("genre", genres_rust, genres_cpp, {nom: nom}, echecs)

    compare(
        "touche",
        touches_rust,
        touches_cpp,
        {
            "CARACTERE": "ToucheCaractere",
            "ENTREE": "ToucheEntree",
            "RETOUR": "ToucheRetour",
            "TABULATION": "ToucheTabulation",
            "HAUT": "ToucheHaut",
            "BAS": "ToucheBas",
            "GAUCHE": "ToucheGauche",
            "DROITE": "ToucheDroite",
            "ECHAP": "ToucheEchap",
        },
        echecs,
    )

    if echecs:
        print()
        for echec in echecs:
            print(f"  ECHEC  {echec}")
        print(
            "\nLes deux cotes du protocole GUI ne sont plus d'accord."
            "\nVoir docs/GUI_USERLAND_PROTOCOL.md : le format se change des deux"
            " cotes, ou pas du tout."
        )
        return 1
    print(f"\n{len(genres_rust)} genres de message, deux implementations d'accord.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
