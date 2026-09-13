#!/usr/bin/env python3
"""Garde-fou : un point de terminaison qui s'est TU n'est pas un point au repos.

# Le defaut, lu sur la machine de reference

Releve du 13 septembre 2026, 11:24. La souris produit ses rapports normalement
puis s'arrete net :

    t= 18.81 s  polls=3296  events=692  mouse=692  rearms=695
    t= 37.87 s  polls=8060  events=692  mouse=692  rearms=695

Dix-neuf secondes sans un evenement, et quatre minutes d'utilisation ensuite
pendant lesquelles le pointeur n'a plus bouge du tout -- l'utilisateur n'a meme
pas pu cliquer sur « eteindre ». La boucle de scrutation tournait pourtant
(`polls` monte de 250 par seconde) et la cloche de relance etait tiree toutes
les cent vingt-huit scrutations (`kicks` monte). Une sonnette ne reveille pas
un point de terminaison ARRETE.

Personne ne venait a son secours :

* le pont EP0 ne s'occupe que des points qui n'ont JAMAIS parle
  (`evenements == 0`), donc jamais de celui-la ;
* rien ne regardait l'etat du point dans le contexte que le controleur tient
  a jour.

# Ce qui rendait le defaut invisible

Vu du seul compteur d'evenements, une souris immobile et une souris morte se
ressemblent trait pour trait. Le contexte du controleur les separe sans
ambiguite : une souris au repos est `Running` AVEC un TD en attente ; un point
casse est arrete, ou `Running` avec un anneau vide -- c'est-a-dire qu'un
achevement s'est perdu.

# Ce qui est verifie

1. Chaque point retient la date de son dernier evenement.
2. Un chien de garde regarde l'ETAT du point, et pas seulement son silence.
3. Il distingue le repos de la panne par l'etat ET par le pointeur de
   defilement -- sans quoi il relancerait une souris immobile sans fin.
4. Il est borne en nombre de reprises.
5. Sa patience est plus courte que celle de l'enumeration : il tourne le
   verrou du pilote tenu.
6. Le pont EP0 accepte de reprendre un point qui a parle puis s'est tu.
7. La date et le drapeau sont remis a zero des que le point reparle.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
XHCI = RACINE / "src/drivers/usb/xhci_active.rs"


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
    if not XHCI.exists():
        print("  - fichier absent : %s" % XHCI)
        return 1
    source = XHCI.read_text(encoding="utf-8")
    fautes = []

    # --- 1. la date du dernier evenement existe et est POSEE ----------------
    if "dernier_evenement_ns" not in source:
        fautes.append(
            "xhci_active.rs : les points de terminaison ne retiennent plus la "
            "date de leur dernier evenement. Sans elle, un point mort et un "
            "point au repos sont indiscernables -- c'est exactement ce qui a "
            "laisse la souris morte pendant quatre minutes le 13 septembre."
        )
    else:
        evenement = corps(source, "fn process_hid_event(")
        if evenement is None:
            fautes.append("xhci_active.rs : process_hid_event a disparu.")
        elif "dernier_evenement_ns =" not in evenement:
            fautes.append(
                "xhci_active.rs : la date du dernier evenement n'est plus "
                "posee a la reception. Le chien de garde croirait le point "
                "muet depuis toujours et le relancerait sans fin."
            )
        elif "interrupt_in_casse = false" not in evenement:
            fautes.append(
                "xhci_active.rs : un point qui REPARLE ne reprend plus son "
                "transport Interrupt-IN ; il resterait sur le pont EP0 pour "
                "le reste de la session."
            )

    # --- 2, 3, 4. le chien de garde -----------------------------------------
    veille = corps(source, "fn veille_points_hid(")
    if veille is None:
        fautes.append(
            "xhci_active.rs : le chien de garde des points HID a disparu. Un "
            "point de terminaison qui s'arrete ne serait plus jamais remis en "
            "marche, et le pointeur resterait mort."
        )
    else:
        if "etat_point_hid(" not in veille:
            fautes.append(
                "xhci_active.rs : le chien de garde ne lit plus l'ETAT du "
                "point. Le silence seul ne dit rien : une souris immobile se "
                "tait aussi."
            )
        # LA CONDITION QUI DISTINGUE LE REPOS DE LA PANNE.
        #
        # Sans elle, chaque souris immobile serait rearmee toutes les trois
        # cents millisecondes, et chaque rearmement pose un TD de plus.
        # LE TEST, ET PAS SEULEMENT LES MOTS.
        #
        # Une premiere version de cette regle cherchait la presence de
        # `EP_ETAT_RUNNING` et de `file_vide` dans la fonction. Elle laissait
        # passer la suppression du test lui-meme : les deux noms survivent
        # plus bas, dans la ligne de journal et dans le choix de la reprise.
        # Ce qui doit exister, c'est le `continue` qui epargne une souris
        # simplement immobile.
        repos = re.search(
            r"etat\s*==\s*EP_ETAT_RUNNING\s*&&\s*!\s*file_vide\s*\{\s*continue",
            veille,
        )
        if repos is None:
            fautes.append(
                "xhci_active.rs : le chien de garde ne distingue plus le repos "
                "de la panne. Une souris immobile est `Running` avec un TD en "
                "attente : la relancer remplirait son anneau de TD jamais "
                "consommes."
            )
        if "REPRISES_SILENCE_MAX" not in veille:
            fautes.append(
                "xhci_active.rs : les reprises du chien de garde ne sont plus "
                "bornees."
            )

    m = re.search(r"const SILENCE_HID_NS: u64 = ([0-9_]+);", source)
    if m is None:
        fautes.append("xhci_active.rs : le seuil de silence n'est plus lisible.")
    else:
        silence = int(m.group(1).replace("_", ""))
        if silence < 50_000_000:
            fautes.append(
                "xhci_active.rs : le seuil de silence est descendu sous "
                "cinquante millisecondes ; il declencherait sur le creux "
                "normal entre deux rapports."
            )
        if silence > 2_000_000_000:
            fautes.append(
                "xhci_active.rs : le seuil de silence depasse deux secondes. "
                "Un pointeur mort pendant deux secondes est un pointeur mort."
            )

    # --- 5. la patience du chien de garde -----------------------------------
    reprise = corps(source, "fn recupere_point_hid(")
    if reprise is None:
        fautes.append("xhci_active.rs : recupere_point_hid a disparu.")
    elif "BUDGET_REPRISE_NS" not in reprise:
        fautes.append(
            "xhci_active.rs : la reprise d'un point reprend la patience de "
            "l'enumeration. Elle tourne le verrou du pilote tenu : une demi-"
            "seconde par tentative ratee, c'est l'entree gelee -- le mal "
            "qu'elle est censee guerir."
        )
    m = re.search(r"const BUDGET_REPRISE_NS: u64 = ([0-9_]+);", source)
    if m is not None and int(m.group(1).replace("_", "")) > 100_000_000:
        fautes.append(
            "xhci_active.rs : la patience d'une reprise depasse cent "
            "millisecondes, verrou du pilote tenu."
        )

    # --- 6. le pont EP0 reprend un point casse ------------------------------
    pont = corps(source, "fn repli_ep0_un_point(")
    if pont is None:
        fautes.append("xhci_active.rs : repli_ep0_un_point a disparu.")
    elif re.search(r"!\s*controller\.hids\[[^\]]+\]\.interrupt_in_casse", pont) is None:
        fautes.append(
            "xhci_active.rs : le pont EP0 refuse de nouveau d'aider un point "
            "qui a parle puis s'est tu -- c'est-a-dire precisement le "
            "peripherique qui avait PROUVE qu'il fonctionnait."
        )

    if fautes:
        print("veille des points HID : %d probleme(s)" % len(fautes))
        for faute in fautes:
            print("\n  - %s" % faute)
        return 1

    print(
        "veille des points HID : silence date, etat verifie, repos distingue "
        "de la panne, reprises bornees et courtes, pont EP0 en dernier recours"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
