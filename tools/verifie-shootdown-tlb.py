#!/usr/bin/env python3
"""Verifie que l'attente d'acquittement du shootdown TLB peut PROGRESSER.

CE QUI S'EST PASSE
------------------
L'emetteur diffusait le vecteur une seule fois, puis attendait :

    send_all_excluding_self(TLB_SHOOTDOWN_VECTOR);
    while slot.acknowledgements.load(Acquire) & targets != targets {
        spin_loop();
    }

Sans borne, sans reemission, sans trace. Cette boucle suppose que l'IPI
arrive toujours. Il ne l'est pas : un CPU dans sa fenetre `cli`
d'endormissement, ou deux IPI de meme vecteur fusionnes par l'APIC, et
l'acquittement ne vient jamais.

La machine s'est figee 285 secondes dans un `munmap`, sans gros verrou, sans
faute de page, sans un seul message -- jusqu'a ce qu'on aille lire les trois
nombres du creneau :

    cpu=0 tlb_cibles=0x2 tlb_acks=0x0 idle=false
    cpu=1                             idle=true

CE QUI EST VERIFIE
------------------
  1. La boucle d'attente REEMET vers les cibles manquantes. Reemettre est
     sur -- le gestionnaire ignore un creneau deja acquitte --, tandis que ne
     pas reemettre coute la machine.
  2. La reemission VISE un CPU precis. Rediffuser a tous reveillerait ceux
     qui ont deja repondu, et masquerait quel coeur manque.

  3. Une panne persistante est FAIL-CLOSED : panique explicite avec le masque
     manquant. Le noyau ne continue jamais avec une traduction perimee, mais il
     ne se fige pas non plus sans diagnostic pendant des minutes.

Code de retour : 0 si l'attente peut progresser.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
SMP = RACINE / "src" / "arch" / "x86_64" / "smp.rs"


def sans_commentaires(texte: str) -> str:
    return "\n".join(ligne.split("//")[0] for ligne in texte.splitlines())


def main() -> int:
    if not SMP.exists():
        print(f"ECHEC  {SMP} introuvable : ce garde-fou ne verifie plus rien.")
        return 1
    texte = sans_commentaires(SMP.read_text(encoding="utf-8"))

    # La zone d'attente est ce qui separe la diffusion de la liberation du
    # creneau. On l'ancre sur ces deux bornes plutot que sur la forme de la
    # boucle : cette forme doit pouvoir changer, l'exigence non.
    attente = re.search(
        r"send_all_excluding_self\(TLB_SHOOTDOWN_VECTOR(.{0,2500}?)slot\.sequence\.store\(0",
        texte,
        re.S,
    )
    if not attente:
        print("ECHEC  zone d'attente des acquittements introuvable entre la\n"
              "       diffusion et la liberation du creneau.\n"
              "       Mets ce garde-fou a jour plutot que de le laisser passer\n"
              "       sur du code qu'il ne lit plus.")
        return 1
    if "acknowledgements" not in attente.group(1):
        print("ECHEC  la zone d'attente ne lit plus les acquittements.")
        return 1

    fautes = []
    corps = attente.group(1)
    if "renvoie_shootdown" not in corps:
        fautes.append(
            "l'attente ne reemet rien.\n"
            "    Un seul IPI perdu et l'emetteur y reste pour toujours, en\n"
            "    silence. C'est exactement la regression mm-ng6."
        )
    if not (
        "ECHEC_SHOOTDOWN_NS" in texte
        and "[TLB-SHOOTDOWN-ECHEC]" in texte
        and "panic!(" in corps
    ):
        fautes.append(
            "l'absence persistante d'ACK n'est plus fail-closed.\n"
            "    Il faut paniquer avec le masque manquant, jamais continuer\n"
            "    avec un TLB perime ni tourner indefiniment en silence."
        )

    emission = re.search(r"fn renvoie_shootdown\(cpu: usize\)(.*?)\n}", texte, re.S)
    if not emission:
        fautes.append("`renvoie_shootdown` a disparu : plus rien ne peut reemettre.")
    elif "send_all_excluding_self" in emission.group(1):
        fautes.append(
            "`renvoie_shootdown` rediffuse a tous au lieu de viser un CPU.\n"
            "    Cela reveille ceux qui ont deja repondu et masque lequel manque."
        )

    if fautes:
        print("ECHEC  l'attente d'acquittement du shootdown TLB ne peut pas progresser\n")
        for faute in fautes:
            print(f"  {faute}\n")
        return 1

    print("ok  shootdown TLB : reemission ciblee et echec persistant fail-closed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
