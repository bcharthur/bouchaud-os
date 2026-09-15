#!/usr/bin/env python3
"""Garde-fou : un clavier muet ne doit pas s'annoncer vert.

# Le defaut, lu sur la machine de reference

Releve du 15 septembre 2026. L'utilisateur ne peut taper nulle part -- ni dans
Ladybird, ni dans le terminal -- et la LED du clavier reste eteinte. Le
diagnostic, lui, annonce que tout va bien :

    BOUCHAUD_HID_CONTROL_FALLBACK_GREEN kind=keyboard
    BOUCHAUD_INPUT_GREEN keyboard=1 mouse=1
    [USB-HID-V3] keyboards=2 mice=1

pendant que la meme session dit, quelques lignes plus bas :

    [GUI-COMPOSITOR-SOURCES] clavier=0 souris=1462
    [GUI-INPUT] touches_client=0 touches_bureau=0
    events=1463 reports=1463 mouse=1463     <- TOUT vient de la souris

Deux claviers enumeres, zero touche, et un vert affiche.

# La cause

« Accepte » voulait dire « analyse sans erreur ». Un clavier au repos rend un
rapport parfaitement valide ou aucune touche n'est enfoncee : il comptait donc
comme accepte, et le premier rapport VIDE suffisait a imprimer le vert. Le pont
EP0 interrogeait ce clavier soixante-quinze fois par seconde en croyant porter
de l'entree.

Les deux notions servent a deux choses differentes et ne peuvent pas partager
un booleen :

    analyse   le transfert a abouti et le rapport se decode. C'est ce qui
              decide de la QUARANTAINE -- un clavier au repos fonctionne et ne
              doit jamais y entrer.
    entree    le rapport a produit au moins un evenement. C'est ce qui a le
              droit d'annoncer un transport vert.

# Ce qui est verifie

1. Les deux notions restent distinctes.
2. Le vert n'est imprime que sur une entree reelle.
3. La quarantaine reste decidee par l'analyse, pas par l'entree.
4. Les temoins du clavier sont poses -- seul signe visible, sans console
   serie, que le chemin de controle atteint l'interface clavier.
5. Le releve sort un etat PAR POINT de terminaison : les compteurs globaux
   additionnent des points qui n'ont pas le meme sort, et c'est cette somme
   qui a cache le defaut.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
XHCI = RACINE / "src/drivers/usb/xhci_active.rs"
WM = RACINE / "src/gui/window_manager.rs"


def corps(source, entete):
    debut = source.find(entete)
    if debut < 0:
        return None
    i = source.find("{", debut)
    if i < 0:
        return None
    profondeur = 0
    for j in range(i, len(source)):
        if source[j] == "{":
            profondeur += 1
        elif source[j] == "}":
            profondeur -= 1
            if profondeur == 0:
                return source[i:j + 1]
    return None


def main():
    fautes = []
    for chemin in (XHCI, WM):
        if not chemin.exists():
            print("  - fichier absent : %s" % chemin)
            return 1
    xhci = XHCI.read_text(encoding="utf-8")
    wm = WM.read_text(encoding="utf-8")
    # LES COMMENTAIRES CITENT LES MARQUEURS, ET C'EST VOULU.
    #
    # Une premiere version de ce controle cherchait les marqueurs dans le
    # fichier entier : elle tombait sur la citation qui EXPLIQUE le defaut,
    # dans le commentaire, au lieu du `serial_println!` qui l'imprime.
    # Chercher dans le code seul est la seule facon de controler le code.
    xhci_code = "\n".join(
        l for l in xhci.splitlines() if not l.lstrip().startswith("//")
    )

    # --- 1. Les deux notions sont distinctes --------------------------------
    if "enum Verdict" not in xhci_code:
        fautes.append(
            "xhci_active.rs : « analyse » et « entree » sont redevenues un "
            "seul booleen. Un clavier au repos rend un rapport valide et vide : "
            "il se compterait de nouveau comme une entree."
        )
    else:
        for fonction in ("fn process_keyboard_report(", "fn process_mouse_report("):
            entete = xhci[xhci.find(fonction):xhci.find(fonction) + 200]
            if "-> Verdict" not in entete:
                fautes.append(
                    "xhci_active.rs : %s ne rend plus de verdict distinct." % fonction
                )
        clavier = corps(xhci, "fn process_keyboard_report(")
        if clavier is not None and "Verdict::Analyse" not in clavier:
            fautes.append(
                "xhci_active.rs : un rapport clavier VIDE ne se distingue plus "
                "d'un rapport qui porte une touche."
            )

    # --- 2. Le vert n'appartient qu'a une entree reelle ---------------------
    #
    # LA REGLE QUI COMPTE. Elle est ce qui separe un diagnostic d'un mensonge.
    for marqueur in (
        "BOUCHAUD_HID_KEYBOARD_INPUT_GREEN",
        "BOUCHAUD_HID_MOUSE_INPUT_GREEN",
        "BOUCHAUD_HID_CONTROL_FALLBACK_GREEN",
    ):
        positions = [
            m.start() for m in re.finditer(re.escape(marqueur), xhci_code)
        ]
        if not positions:
            fautes.append("xhci_active.rs : le marqueur %s a disparu." % marqueur)
            continue
        # CHAQUE impression doit etre gardee par une entree reelle, pas
        # seulement la premiere.
        for position in positions:
            amont = xhci_code[max(0, position - 600):position]
            if "entree()" not in amont:
                fautes.append(
                    "xhci_active.rs : %s n'est plus garde par une entree "
                    "REELLE. Le premier rapport vide le rallumerait, et c'est "
                    "exactement le vert mensonger du 15 septembre." % marqueur
                )
                break

    # --- 3. La quarantaine reste decidee par l'analyse ----------------------
    pont = corps(xhci, "fn control_get_report(")
    if pont is None:
        fautes.append("xhci_active.rs : control_get_report a disparu.")
    # LA VALEUR DE RETOUR, ET PAS SEULEMENT SA PRESENCE.
    #
    # Une premiere version cherchait `verdict.analyse()` n'importe ou dans le
    # corps : elle laissait passer le remplacement du RETOUR par
    # `verdict.entree()`, alors que le reste du corps mentionne encore
    # l'analyse. C'est le retour qui decide de la quarantaine.
    elif re.search(r"verdict\.analyse\(\)\s*\}\s*$", pont) is None:
        fautes.append(
            "xhci_active.rs : le pont EP0 ne REND plus l'analyse. S'il rendait "
            "l'entree, un clavier au repos -- qui fonctionne -- partirait en "
            "quarantaine a son premier rapport vide."
        )

    # --- 4. Les temoins du clavier sont poses -------------------------------
    temoins = corps(xhci, "fn set_temoins_clavier(")
    if temoins is None:
        fautes.append(
            "xhci_active.rs : le pilote ne sait plus allumer les temoins du "
            "clavier. C'est le seul signe VISIBLE, sans console serie, que le "
            "chemin de controle atteint l'interface clavier."
        )
    else:
        if "0x09" not in temoins:
            fautes.append(
                "xhci_active.rs : les temoins ne passent plus par SET_REPORT "
                "(requete de classe 0x09, HID 1.11 section 7.2.2)."
            )
        if "control_transfer_sortant(" not in temoins:
            fautes.append(
                "xhci_active.rs : les temoins n'emettent plus de charge "
                "sortante ; le rapport partirait vide."
            )
    if "set_temoins_clavier(controller, device, interface" not in xhci_code:
        fautes.append(
            "xhci_active.rs : les temoins ne sont plus poses a l'installation "
            "d'un clavier."
        )

    sortant = corps(xhci, "fn control_transfer_sortant(")
    if sortant is None:
        fautes.append("xhci_active.rs : le transfert de controle sortant a disparu.")
    interne = corps(xhci, "fn control_transfer_interne(")
    if interne is None or "charge_sortante" not in (interne or ""):
        fautes.append(
            "xhci_active.rs : le tampon de controle est de nouveau efface sans "
            "qu'on puisse y ecrire ; aucun rapport de sortie ne partirait."
        )

    # --- 5. Un etat PAR POINT de terminaison --------------------------------
    if "fn pour_chaque_point_hid(" not in xhci_code:
        fautes.append(
            "xhci_active.rs : le releve par point de terminaison a disparu. Les "
            "compteurs globaux additionnent des points qui n'ont pas le meme "
            "sort : sur le meme peripherique, le point souris marchait et le "
            "point clavier non, et la somme cachait le fait."
        )
    if "[USB-HID-POINT]" not in wm:
        fautes.append(
            "window_manager.rs : le releve periodique ne sort plus l'etat de "
            "chaque point HID."
        )
    else:
        ligne = wm[wm.find("[USB-HID-POINT]"):wm.find("[USB-HID-POINT]") + 700]
        for champ in ("evenements=", "silence_ms=", "etat=", "deq=", "attendu="):
            if champ not in ligne:
                fautes.append(
                    "window_manager.rs : le releve par point ne publie plus "
                    "`%s`, sans quoi on ne peut pas distinguer un clavier au "
                    "repos d'un point de terminaison mort." % champ
                )

    if fautes:
        print("entree clavier : %d probleme(s)" % len(fautes))
        for faute in fautes:
            print("\n  - %s" % faute)
        return 1

    print(
        "entree clavier : analyse et entree distinctes, vert reserve a une "
        "entree reelle, temoins poses, etat par point de terminaison"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
