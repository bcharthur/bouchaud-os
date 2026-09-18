#!/usr/bin/env python3
"""Garde-fou : une reprise NE DESACTIVE PAS le pilote.

# Le defaut, releve en archive et en photo le 18 septembre 2026

`bb(6)`, TRIGKEY, commit `76cf28d` :

    t=43,7 s   rx_packets=64  rx_cur=0        un tour d'anneau, puis plus rien
    t=78..81   recoveries 1, 2, 3             le chien de garde fait son travail
    t=83,3 s   TOUT a zero, chip_cmd=0x00     la carte est morte

La reprise de degre quatre faisait ceci :

    READY = false;
    MMIO = 0;
    return false;

Ce n'est pas une reinitialisation, c'est une desactivation definitive. Et
comme le modele de carte etait choisi d'apres `rtl8168::is_ready()`, l'ecran
s'est mis a annoncer :

    net.nic.rtl8168   Indisponible   absente de ce materiel
    net.nic.e1000     Erreur         carte non pilotee

pour une puce soudee qui venait de recevoir soixante-quatre trames, et pour
une carte qui n'a jamais existe sur cette machine.

# Les invariants defendus ici

1. La reprise ne pose plus `MMIO = 0` : sans le BAR, plus rien ne sait ou est
   la carte, et le pilote ne peut que se declarer absent.
2. Le degre quatre reprogramme le materiel (`programme_le_materiel`) au lieu
   de renoncer.
3. Les anneaux DMA ne se reallouent pas a chaque reprise.
4. Le modele de carte publie vient de la PRESENCE PCI, jamais de l'etat du
   pilote.
5. Le rearmement d'un descripteur passe par une vraie barriere DMA.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
PILOTE = RACINE / "src/drivers/network/rtl8168.rs"
SERVICES = RACINE / "src/kernel/services/mod.rs"

SIGNATURE_REPARE = "unsafe fn repare_reception() -> bool {"
SIGNATURE_PROGRAMME = "unsafe fn programme_le_materiel() -> bool {"
SIGNATURE_RECEIVE = "pub fn receive(out: &mut [u8]) -> Option<usize> {"
SIGNATURE_CARTE = "pub fn carte_active() -> &'static str {"


def code_seul(source):
    """Le code sans les commentaires : cette garde s'accuserait elle-meme,
    puisque la prose du pilote cite le defaut qu'elle defend."""
    sans = re.sub(r"/\*.*?\*/", "", source, flags=re.S)
    return "\n".join(re.sub(r"//.*$", "", l) for l in sans.splitlines())


def corps(source, signature):
    d = source.find(signature)
    if d < 0:
        return None
    reste = source[d + len(signature):]
    fin = re.search(r"\n\s*(?:pub )?(?:unsafe )?(?:const |static |fn |struct |impl |enum |mod )", reste)
    return reste[: fin.start()] if fin else reste


def main():
    fautes = []
    try:
        pilote = code_seul(PILOTE.read_text(encoding="utf-8", errors="replace"))
        services = code_seul(SERVICES.read_text(encoding="utf-8", errors="replace"))
    except OSError as erreur:
        print("  - source illisible : %s" % erreur)
        return 1

    # ------------------------------------------------------------------ 1
    corps_repare = corps(pilote, SIGNATURE_REPARE)
    if corps_repare is None:
        fautes.append("`%s` est introuvable." % SIGNATURE_REPARE)
    else:
        if re.search(r"\bMMIO\s*=\s*0\b", corps_repare):
            fautes.append(
                "la reprise pose `MMIO = 0` : sans le BAR, plus rien ne sait "
                "ou est la carte, et le pilote ne peut que se declarer absent."
            )
        if re.search(r"\bREADY\s*=\s*false\b", corps_repare):
            fautes.append(
                "la reprise pose `READY = false` directement : l'etat du "
                "pilote doit passer par `pose_etat`, qui refuse le retour a "
                "`Absent`."
            )
        if "programme_le_materiel" not in corps_repare:
            fautes.append(
                "le degre le plus haut ne reprogramme plus le materiel : "
                "c'est une desactivation, pas une reinitialisation."
            )

    # ------------------------------------------------------------------ 2
    corps_programme = corps(pilote, SIGNATURE_PROGRAMME)
    if corps_programme is None:
        fautes.append("`%s` est introuvable." % SIGNATURE_PROGRAMME)
    elif "alloc_dma" in corps_programme:
        fautes.append(
            "`programme_le_materiel` realloue des anneaux DMA : une carte qui "
            "tombe toutes les minutes epuiserait la memoire basse."
        )

    # ------------------------------------------------------------------ 3
    corps_receive = corps(pilote, SIGNATURE_RECEIVE)
    if corps_receive is None:
        fautes.append("`%s` est introuvable." % SIGNATURE_RECEIVE)
    else:
        if "dma_wmb" not in corps_receive:
            fautes.append(
                "le rearmement d'un descripteur n'utilise pas `dma_wmb` : un "
                "`compiler_fence` n'est pas une barriere DMA, il n'ordonne "
                "que le compilateur."
            )
        if "dma_rmb" not in corps_receive:
            fautes.append(
                "la lecture d'un descripteur rendu n'utilise pas `dma_rmb`."
            )

    # ------------------------------------------------------------------ 4
    corps_carte = corps(services, SIGNATURE_CARTE)
    if corps_carte is None:
        fautes.append("`%s` est introuvable." % SIGNATURE_CARTE)
    else:
        if "using_rtl8168" in corps_carte or "is_ready" in corps_carte:
            fautes.append(
                "le modele de carte est choisi d'apres l'etat du pilote : une "
                "carte ne change pas de modele parce que son pilote a "
                "renonce. Il vient de la presence PCI."
            )
        if "presente()" not in corps_carte:
            fautes.append(
                "le modele de carte ne vient pas de la presence PCI "
                "(`rtl8168::presente`)."
            )

    # ------------------------------------------------------------------ 5
    #
    # Un premier tour d'anneau ne prouve rien : le compteur doit exister.
    for compteur in ["RX_TOURS_CPU", "RX_REUTILISES_TOUR2", "RX_OK_SANS_PROGRES"]:
        if compteur not in pilote:
            fautes.append(
                "le compteur `%s` a disparu : sans lui, un unique tour "
                "d'anneau se lit comme une reception qui fonctionne." % compteur
            )

    if fautes:
        print("PILOTE RTL8168 : %d manquement(s)" % len(fautes))
        for f in fautes:
            print("  - %s" % f)
        return 1

    print("pilote RTL8168 : la reprise reprogramme, elle ne desactive pas.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
