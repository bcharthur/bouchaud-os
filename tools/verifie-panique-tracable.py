#!/usr/bin/env python3
"""Garde-fou : une panique noyau doit laisser une trace la ou elle sera lue.

# Le defaut, photographie le 17 septembre 2026

Une panique au lancement du navigateur -- `library/alloc/src/alloc.rs`, ligne
0x23D, c'est-a-dire une allocation refusee. L'ecran affichait huit lignes, la
derniere disant :

    LE DETAIL COMPLET EST SUR COM1.

La machine de reference n'a pas de COM1. `presence_com1()` y rend
`bus-flottant`, et le noyau le sait depuis des mois. Tout le releve -- le
message avec la TAILLE demandee, le contexte, l'enregistreur de vol -- partait
donc vers un port qui n'existe pas.

Et l'archive de la cle ne contenait pas un mot de la session : le handler de
panique n'ecrivait rien dans l'enregistreur. Quatre sessions passees a le
rendre increvable, et le seul evenement qu'il devait absolument retenir ne lui
etait jamais confie.

C'est la meme faute que « /persist n'est PAS a jour » sur une machine sans
zone de persistance : annoncer un endroit qui n'existe pas fait perdre le
temps de l'enquete, et parfois l'enquete elle-meme.

# Ce qui est verifie ici

1. Le handler de panique confie son releve a l'enregistreur.
2. Il formate son message SANS allouer -- il y arrive parfois parce que
   l'allocation a echoue, et rappeler l'allocateur ferait paniquer dans le
   handler de panique, ou plus rien ne s'affiche.
3. L'ecran porte le message : c'est lui qui contient la taille demandee.
4. L'ecran dit ou est le detail SELON ce que la machine a vraiment.
5. `blackbox::panique` pose en memoire AVANT de tenter le support, et son
   vidage est borne.
6. Il releve l'etat du tas : une panique d'allocation sans lui laisse la meme
   question ouverte qu'un ecran vide.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
PANIC = RACINE / "src/kernel/debug/panic.rs"
ECRAN = RACINE / "src/platform/pc/ecran_faute.rs"
BB = RACINE / "src/kernel/debug/blackbox.rs"


def corps(source, signature):
    d = source.find(signature)
    if d < 0:
        return None
    reste = source[d + len(signature):]
    fin = re.search(r"\n\s*(?:pub )?(?:const |static |fn |struct |impl |enum )", reste)
    return reste[: fin.start()] if fin else reste


def lit(chemin, fautes):
    if not chemin.exists():
        fautes.append("fichier absent : %s" % chemin.relative_to(RACINE).as_posix())
        return None
    return chemin.read_text(encoding="utf-8")


def main():
    fautes = []
    panic = lit(PANIC, fautes)
    ecran = lit(ECRAN, fautes)
    bb = lit(BB, fautes)

    if panic is not None:
        handler = corps(panic, "fn panic(info: &PanicInfo) -> ! {")
        if handler is None:
            fautes.append("panic.rs : le handler de panique est introuvable.")
        else:
            # 1. Le releve part vers l'enregistreur.
            if "blackbox::panique" not in handler:
                fautes.append(
                    "panic.rs : la panique ne confie plus rien a l'enregistreur. "
                    "Sur une machine sans COM1 -- celle de reference --, elle ne "
                    "laisserait que huit lignes a l'ecran, et l'archive pas un "
                    "mot. C'est exactement ce qui s'est passe le 17 septembre."
                )
            # L'ordre : l'ecran d'abord, la cle en dernier. Une panique qui
            # tenterait la cle avant d'afficher risquerait de n'afficher jamais.
            ecr = handler.find("affiche_panique")
            cle = handler.find("blackbox::panique")
            if ecr >= 0 and cle >= 0 and cle < ecr:
                fautes.append(
                    "panic.rs : la cle est tentee AVANT l'ecran. Un support qui "
                    "ne repond pas couterait alors l'affichage, qui est la seule "
                    "chose qu'on ait toujours."
                )
        # 2. Le message se formate sans allouer.
        if "MessagePanique" not in panic:
            fautes.append(
                "panic.rs : le message n'est plus formate sur la pile. Une "
                "panique d'allocation rappellerait l'allocateur qui vient "
                "d'echouer, et paniquerait dans le handler de panique."
            )
        for interdit in ("String::", "format!", "vec!", "to_string()"):
            if interdit in panic:
                fautes.append(
                    "panic.rs : `%s` alloue. Le chemin de panique y arrive "
                    "parfois PARCE QUE l'allocation a echoue." % interdit
                )

    if ecran is not None:
        affiche = corps(ecran, "pub fn affiche_panique(")
        if affiche is None:
            fautes.append("ecran_faute.rs : `affiche_panique` est introuvable.")
        else:
            # 3. Le message est affiche.
            if "message" not in affiche:
                fautes.append(
                    "ecran_faute.rs : l'ecran de panique ne porte plus le "
                    "message. C'est lui qui contient la taille demandee quand "
                    "une allocation echoue -- le chiffre le plus utile de "
                    "l'ecran."
                )
            # 4. Il dit ou est le detail, selon la machine.
            if "presence_com1" not in affiche:
                fautes.append(
                    "ecran_faute.rs : l'ecran annonce un endroit sans verifier "
                    "qu'il existe. « LE DETAIL COMPLET EST SUR COM1 » sur une "
                    "machine qui n'a pas de COM1 envoie chercher la reponse la "
                    "ou elle n'est pas."
                )

    if bb is not None:
        pan = corps(bb, "pub fn panique(")
        if pan is None:
            fautes.append("blackbox.rs : `panique` est introuvable.")
        else:
            # 5. La memoire d'abord, le support ensuite et borne.
            memoire = pan.find("append(KIND_FATAL")
            support = pan.find("blackbox_storage_ready")
            if memoire < 0:
                fautes.append(
                    "blackbox.rs : `panique` ne pose plus l'enregistrement en "
                    "memoire. C'est la seule pose qui ne peut pas echouer."
                )
            elif support >= 0 and support < memoire:
                fautes.append(
                    "blackbox.rs : `panique` consulte le support AVANT de poser "
                    "en memoire. Une cle absente couterait alors la trace "
                    "entiere, alors qu'elle ne devrait rien couter du tout."
                )
            if "BUDGET_FATAL_NS" not in pan:
                fautes.append(
                    "blackbox.rs : le vidage de panique n'est plus borne. Une "
                    "panique qui attendrait sans fin remplacerait un diagnostic "
                    "par un ecran noir."
                )
            # 6. L'etat du tas voyage avec.
            if "PANIC_TAS" not in pan:
                fautes.append(
                    "blackbox.rs : l'etat du tas ne voyage plus avec la "
                    "panique. Une allocation refusee sans lui laisse la meme "
                    "question ouverte qu'un ecran vide : manquait-il un octet "
                    "ou un mebioctet ?"
                )

    if fautes:
        for f in fautes:
            print("FAUTE: %s" % f)
        return 1
    print(
        "panique tracable : releve confie a l'enregistreur, message sans "
        "allocation, ecran qui porte le message et dit ou chercher, vidage "
        "borne avec l'etat du tas."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
