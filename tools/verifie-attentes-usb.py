#!/usr/bin/env python3
"""Garde-fou : une attente USB se borne en TEMPS, et la scrutation n'attend pas.

# Le defaut, mesure sur la machine de reference

`WAIT_SPINS` valait 30 000 000 tours de `pause`. Un `pause` coute une
trentaine de cycles sur un Zen 3 : l'attente complete valait donc environ
330 ms sur le Ryzen 7 5800H du TRIGKEY, cadence a 3,194 GHz.

Ce n'etait pas une borne theorique jamais atteinte. `poll_control_fallback`
emet un `GET_REPORT` sur EP0, un tour sur deux, pour CHAQUE point de
terminaison muet en Interrupt-IN -- et le recepteur Logitech du TRIGKEY expose
une interface vendeur (`if=2 subclass=0 protocol=0`) qui n'a aucune raison d'y
repondre. Une attente non aboutie par tour de scrutation.

L'enregistreur de vol a mesure le resultat :

    hid polls=3.19/s        une scrutation toutes les 313 ms
    330 ms predits, 313 ms mesures

Tout ce qui depend de l'entree avancait a 3 Hz : le pointeur, les frappes, et
le compositeur qui n'est reveille que par elles. La machine etait inutilisable,
et aucun scenario QEMU ne pouvait le voir -- QEMU repond instantanement, et n'a
pas d'interface muette.

# Ce qui est verifie

1. Aucune attente du pilote xHCI n'est bornee par un compte de tours. Une
   constante de tours ne dit rien : la meme valeur vaut 30 ms sur une machine
   et 330 ms sur une autre.
2. Le chemin de SCRUTATION a son propre budget, court, et distinct de celui de
   l'enumeration. Les confondre est exactement ce qui est arrive.
3. Un point de terminaison qui ne repond pas au repli est mis en QUARANTAINE :
   le redemander mille fois par seconde ne le reveille pas, cela paie son
   echeance mille fois par seconde.
4. L'enregistreur de vol n'ecrit PAS depuis le chemin d'entree. Sur le TRIGKEY
   la cle de demarrage est sa cible : chaque scrutation payait ses transferts
   Bulk synchrones.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
XHCI = RACINE / "src/drivers/usb/xhci_active.rs"


def sans_commentaires(texte):
    return "\n".join(
        ligne for ligne in texte.splitlines()
        if not ligne.lstrip().startswith("//")
    )


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
        print("  - fichier absent : src/drivers/usb/xhci_active.rs")
        return 1
    source = sans_commentaires(XHCI.read_text(encoding="utf-8"))
    fautes = []

    # --- 1. plus aucune borne en nombre de tours -----------------------------
    if "WAIT_SPINS" in source:
        fautes.append(
            "xhci_active.rs : une attente est de nouveau bornee par un compte "
            "de tours. Trente millions de `pause` valaient 330 ms sur le "
            "TRIGKEY, payees a chaque scrutation. Une borne en nanosecondes "
            "vaut ce qu'elle annonce ; un compte de tours ne vaut rien."
        )
    for nom in ("BUDGET_ATTENTE_NS", "BUDGET_SCRUTATION_NS"):
        if nom not in source:
            fautes.append(
                "xhci_active.rs : le budget `%s` a disparu ; l'attente n'est "
                "plus bornee en temps." % nom
            )

    # Le budget de scrutation doit rester COURT. Dix millisecondes, c'est deja
    # dix fois ce qu'un peripherique HID demande.
    m = re.search(r"const BUDGET_SCRUTATION_NS: u64 = ([0-9_]+);", source)
    if m is None:
        fautes.append("xhci_active.rs : BUDGET_SCRUTATION_NS n'est plus une constante lisible.")
    elif int(m.group(1).replace("_", "")) > 10_000_000:
        fautes.append(
            "xhci_active.rs : le budget de scrutation depasse dix "
            "millisecondes. Il s'execute un tour sur deux, pour chaque point "
            "muet : ce qu'on lui accorde, l'entree le paie."
        )

    # --- 2. l'attente consulte reellement l'horloge --------------------------
    attente = corps(source, "fn wait_event_budget(")
    if attente is None:
        fautes.append("xhci_active.rs : wait_event_budget a disparu.")
    else:
        if "monotonic_ns()" not in attente:
            fautes.append(
                "xhci_active.rs : l'attente ne lit plus l'horloge ; sa borne "
                "n'est plus une duree."
            )
        if "budget_ns" not in attente:
            fautes.append(
                "xhci_active.rs : l'attente n'utilise plus le budget qu'on lui "
                "passe. Un budget ignore est pire qu'absent."
            )

    # --- 3. le repli met les muets en quarantaine ----------------------------
    repli = corps(source, "fn poll_control_fallback(")
    if repli is None:
        fautes.append("xhci_active.rs : poll_control_fallback a disparu.")
    else:
        # LA CONSULTATION, ET PAS SEULEMENT LE NOM.
        #
        # Une premiere version de cette regle cherchait la presence de
        # `repli_muet_jusqu_a_ns` dans la fonction. Elle laissait passer la
        # suppression du test : le champ restait NOMME plus bas, la ou la
        # quarantaine est POSEE. Une quarantaine qu'on pose sans jamais la lire
        # ne quarantaine rien.
        consulte = re.search(
            r"maintenant\s*<\s*controller\.hids\[[^\]]+\]\.repli_muet_jusqu_a_ns",
            repli,
        )
        pose = "repli_muet_jusqu_a_ns =" in repli
        if not consulte:
            fautes.append(
                "xhci_active.rs : le repli EP0 ne CONSULTE plus la quarantaine. "
                "Un point de terminaison muet redeviendrait interroge cinq "
                "cents fois par seconde, et chaque interrogation coute son "
                "echeance."
            )
        if not pose:
            fautes.append(
                "xhci_active.rs : le repli EP0 ne POSE plus de quarantaine ; "
                "un point muet ne serait jamais ecarte."
            )
        if "BUDGET_SCRUTATION_NS" not in source[source.find("fn control_get_report"):
                                                 source.find("fn control_get_report") + 3000]:
            fautes.append(
                "xhci_active.rs : le `GET_REPORT` du repli n'utilise plus le "
                "budget de scrutation ; il reprendrait la patience de "
                "l'enumeration sur un chemin parcouru mille fois par seconde."
            )

    # --- 4. l'enregistreur de vol n'est pas sur le chemin d'entree -----------
    scrutation = corps(source, "pub fn poll()")
    if scrutation is None:
        fautes.append("xhci_active.rs : poll() a disparu.")
    elif "blackbox::poll()" in scrutation:
        fautes.append(
            "xhci_active.rs : l'enregistreur de vol est revenu dans `poll()`. "
            "Sur le TRIGKEY la cle de demarrage est sa cible : chaque "
            "scrutation d'entree paierait ses transferts Bulk synchrones, et "
            "rien de tout cela ne se verrait sous QEMU."
        )
    if "fn fil_blackbox()" not in source:
        fautes.append(
            "xhci_active.rs : le fil propre de l'enregistreur a disparu ; il "
            "est soit muet, soit revenu sur le chemin de l'entree."
        )

    if fautes:
        print("attentes USB : %d probleme(s)\n" % len(fautes))
        for faute in fautes:
            print("  - %s\n" % faute)
        return 1
    print(
        "attentes USB : bornees en temps, budget de scrutation court et "
        "distinct, points muets en quarantaine, enregistreur de vol hors du "
        "chemin d'entree"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
