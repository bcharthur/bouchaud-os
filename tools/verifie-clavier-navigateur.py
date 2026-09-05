#!/usr/bin/env python3
"""Verifie qu'aucune touche du protocole ne s'arrete au bord du chrome.

LE DEFAUT
---------
Le decodeur clavier ne reconnaissait, parmi les sequences etendues, que les
quatre fleches. Origine, Fin, Page precedente, Page suivante et Inser rendaient
`None` : elles n'atteignaient jamais un client. Sur le bureau, cela ne se voyait
pas -- rien ne se passe, et rien est ce qu'on attend d'un octet inconnu. Dans un
navigateur, cela voulait dire qu'une page ne se faisait defiler qu'a la molette.

Suppr etait pire que perdue : `0xE0 0x53` etait traduit en `Key::Backspace`. La
touche arrivait, et effacait le caractere de GAUCHE -- la seule chose qu'elle ne
doit pas faire.

Ces touches traversent maintenant quatre couches : le decodeur PS/2, le
gestionnaire de fenetres, le protocole GUI, le chrome. Chacune peut les laisser
tomber sans que rien ne devienne rouge.

CE QUI GARDE QUOI
-----------------
* Le decodeur : `tools/gui/test_clavier.rs`, qui inclut le fichier reel et
  deroule des scancodes.
* L'accord des trois implementations du protocole sur les VALEURS :
  `tools/verifie-protocole-gui.py`.
* Ce fichier : que le chrome AGISSE sur chacune. Un code declare des deux cotes,
  transporte correctement, et tombant dans un `default:` silencieux passerait les
  deux verificateurs precedents.

LES REGLES
----------
1. Chaque `CodeTouche` declare par le chrome est traite quelque part dans
   `handle_key` -- un `case` ne suffit pas s'il est vide, mais un code sans
   aucun `case` est certainement ignore.

2. Les raccourcis du navigateur existent : rechargement, historique, barre
   d'adresse. Ce sont les trois que les touches nouvellement transportees
   rendent possibles ; sans eux, les transporter n'aurait servi a rien.

3. Les raccourcis sont examines AVANT le foyer. Un raccourci qui ne fonctionne
   que lorsque le curseur est au bon endroit n'est pas un raccourci -- et le
   defaut ne se voit qu'en essayant F5 avec la barre d'adresse active.

4. La selection totale du champ d'adresse ne survit pas au foyer. Une
   surbrillance restee affichee ferait effacer l'URL a la frappe suivante, sans
   que rien ne l'annonce.

Code de retour : 0 si les quatre regles sont respectees.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
CHROME = RACINE / "tools" / "ladybird" / "chrome" / "BouchaudChrome.h"
DECODEUR = RACINE / "src" / "drivers" / "input" / "clavier_decodeur.rs"


def corps_fonction(source, signature):
    debut = source.find(signature)
    if debut < 0:
        return None
    ouvrante = source.find("{", debut)
    if ouvrante < 0:
        return None
    profondeur = 0
    for index in range(ouvrante, len(source)):
        if source[index] == "{":
            profondeur += 1
        elif source[index] == "}":
            profondeur -= 1
            if profondeur == 0:
                return source[ouvrante : index + 1]
    return None


def regle_toutes_traitees(chrome, fautes):
    """1. Aucun code de touche declare puis ignore."""
    bloc = re.search(r"enum CodeTouche : u32 \{(.*?)\n\};", chrome, re.S)
    if not bloc:
        fautes.append("BouchaudChrome.h : enum CodeTouche introuvable.")
        return
    codes = [nom for nom, _ in re.findall(r"(\w+) = (\d+),", bloc.group(1))]

    corps = corps_fonction(chrome, "inline void handle_key(")
    if corps is None:
        fautes.append("BouchaudChrome.h : `handle_key` introuvable.")
        return
    # Les raccourcis sont dans leur propre fonction : un code qui n'est traite
    # que la est traite quand meme.
    raccourcis = corps_fonction(chrome, "inline bool raccourci_navigateur(") or ""
    vu = corps + raccourcis

    for code in codes:
        if code not in vu:
            fautes.append(
                "BouchaudChrome.h : %s est declare dans le protocole mais "
                "n'apparait nulle part dans `handle_key`. La touche traverse "
                "quatre couches pour tomber dans un `default:` muet." % code
            )


def regle_raccourcis(chrome, fautes):
    """2. Les trois raccourcis que ces touches rendent possibles."""
    corps = corps_fonction(chrome, "inline bool raccourci_navigateur(")
    if corps is None:
        fautes.append(
            "BouchaudChrome.h : `raccourci_navigateur` a disparu. Le chrome "
            "n'aurait plus aucun raccourci clavier."
        )
        return
    # L'APPEL, pas le nom : `if (s.on_history_delta)` mentionne le rappel sans
    # rien en faire, et une regle qui se contente du nom se laisse satisfaire
    # par la garde qui precede l'appel supprime.
    for symbole, quoi in (
        ("on_reload(", "rechargement (F5, Ctrl+R)"),
        ("on_history_delta(", "historique (Alt+fleche)"),
        ("focus_address_bar(", "barre d'adresse (Ctrl+L)"),
    ):
        if symbole not in corps:
            fautes.append(
                "BouchaudChrome.h : le raccourci de %s a disparu de "
                "`raccourci_navigateur`." % quoi
            )
    if "ToucheFonction" not in corps:
        fautes.append(
            "BouchaudChrome.h : F5 n'est plus reconnue comme rechargement ; "
            "elle irait au document, ou elle ne veut rien dire."
        )


def regle_avant_le_foyer(chrome, fautes):
    """3. Les raccourcis passent avant la barre d'adresse."""
    corps = corps_fonction(chrome, "inline void handle_key(")
    if corps is None:
        return
    appel = corps.find("raccourci_navigateur(")
    foyer = corps.find("if (s.address_focused)")
    if appel < 0:
        fautes.append(
            "BouchaudChrome.h : `handle_key` n'appelle plus "
            "`raccourci_navigateur`."
        )
        return
    if foyer >= 0 and appel > foyer:
        fautes.append(
            "BouchaudChrome.h : les raccourcis sont examines APRES le foyer. "
            "F5 ne rechargerait plus tant que la barre d'adresse est active, "
            "et un raccourci conditionnel n'est pas un raccourci."
        )


def regle_selection(chrome, fautes):
    """4. La selection totale ne survit pas au foyer."""
    if "address_all_selected" not in chrome:
        fautes.append(
            "BouchaudChrome.h : la selection totale du champ d'adresse a "
            "disparu ; Ctrl+L obligerait a effacer l'URL a la main."
        )
        return
    corps = corps_fonction(chrome, "inline void defocus_address()")
    if corps is None or "address_all_selected = false" not in corps:
        fautes.append(
            "BouchaudChrome.h : rendre le foyer au document ne defait plus la "
            "selection. Une surbrillance survivante ferait effacer l'URL a la "
            "frappe suivante."
        )
    # Les assignations directes contournent `defocus_address()` : c'est
    # exactement ce qui avait disperse l'invariant sur quatre sites.
    directes = len(re.findall(r"address_focused = false", chrome))
    if directes > 1:
        fautes.append(
            "BouchaudChrome.h : %d assignations directes de `address_focused = "
            "false` ; elles contournent `defocus_address()` et laisseraient la "
            "selection derriere elles." % directes
        )


def regle_suppr(decodeur, fautes):
    """Suppr n'est pas Retour arriere. Le defaut le plus vicieux des trois."""
    bloc = re.search(r"if etendue \{(.*?)\n            \}", decodeur, re.S)
    if not bloc:
        bloc = re.search(r"match base \{(.*?)\n            \}", decodeur, re.S)
    if not bloc:
        fautes.append("clavier_decodeur.rs : table des sequences etendues introuvable.")
        return
    corps = bloc.group(1)
    ligne = [l for l in corps.splitlines() if "0x53" in l.split("//", 1)[0]]
    if not ligne:
        fautes.append(
            "clavier_decodeur.rs : Suppr (0xE0 0x53) n'est plus decodee."
        )
        return
    if "Key::Delete" not in ligne[0]:
        fautes.append(
            "clavier_decodeur.rs : Suppr redevient autre chose que "
            "`Key::Delete`.\n           %s" % ligne[0].strip()
        )


def main():
    fautes = []
    chrome = CHROME.read_text(encoding="utf-8")
    decodeur = DECODEUR.read_text(encoding="utf-8")

    regle_toutes_traitees(chrome, fautes)
    regle_raccourcis(chrome, fautes)
    regle_avant_le_foyer(chrome, fautes)
    regle_selection(chrome, fautes)
    regle_suppr(decodeur, fautes)

    if fautes:
        for faute in fautes:
            print("ECHEC  %s" % faute)
        return 1

    print("clavier navigateur : chaque code traite, raccourcis avant le foyer, "
          "selection liee au foyer, Suppr distincte de Retour arriere")
    return 0


if __name__ == "__main__":
    sys.exit(main())
