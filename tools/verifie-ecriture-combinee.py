#!/usr/bin/env python3
"""Garde-fou : le framebuffer est ECRIT EN COMBINE, sur chaque coeur.

# Le defaut, mesure sur la machine de reference

L'enregistreur de vol chronometre chaque presentation :

    zone 50x25 (curseur)      0,03 ms      ~167 Mo/s
    zone 1920x1080 (plein)   75,00 ms      ~110 Mo/s

Le cout est STRICTEMENT proportionnel aux octets ecrits, quelle que soit la
taille du rectangle : ce n'est donc pas un surcout par trame, c'est le debit de
la destination. Cent dix megaoctets par seconde, quand un `memcpy` vers de la
memoire normale en fait dix a vingt MILLE sur ce meme processeur.

Le framebuffer est une fenetre PCIe, que le micrologiciel decrit comme non
cachable dans ses MTRR. Pour des registres, c'est juste. Pour des pixels, cela
coute cent fois le prix -- et cela ne se voit pas tant que le bureau ne
redessine qu'un curseur. L'utilisateur l'a decrit ainsi : « fluide, puis apres
avoir agrandi le terminal, la souris se teleporte ».

# Les deux moities, dont aucune ne suffit

1. La table PAT doit designer l'ecriture combinee a l'indice quatre, SUR
   CHAQUE COEUR. La PAT est par coeur : un coeur oublie voit la meme page non
   cachable, ne faute pas, et rend une trame cent fois plus lente en silence.
2. Les pages du framebuffer doivent designer cet indice.

La premiere version de ce lot avait les deux moities et ne marchait pas : la
configuration vivait dans `arch::x86_64::init()`, que le chemin UEFI du bureau
n'atteint jamais -- `stage2::run` est appele avant et ne rend pas la main. Le
releve le disait pourtant : `coeurs_pat=0 pat=0x0007040600070406`, la valeur
de sortie d'usine.

# Ce qui est verifie

Que la sequence du manuel soit suivie, que les entrees zero a trois soient
laissees intactes, que la configuration soit sur le chemin que TOUS les
demarrages traversent et sur l'entree des coeurs secondaires, que la bascule
des pages refuse une page d'un gibioctet, et qu'une barriere suive chaque
presentation.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
PAT = RACINE / "src/arch/x86_64/pat.rs"
VMM = RACINE / "src/kernel/memory/virtual.rs"
MAIN = RACINE / "src/main.rs"
SMP = RACINE / "src/arch/x86_64/smp.rs"
ECRAN = RACINE / "src/drivers/display/bochs.rs"


def sans_commentaires(texte):
    return "\n".join(
        l for l in texte.splitlines() if not l.lstrip().startswith("//")
    )


def lit(chemin):
    return sans_commentaires(chemin.read_text(encoding="utf-8")) if chemin.exists() else None


def main():
    fautes = []

    pat = lit(PAT)
    if pat is None:
        fautes.append("src/arch/x86_64/pat.rs : absent. Sans PAT, le framebuffer "
                      "reste non cachable et chaque trame plein ecran coute son debit.")
    else:
        # La sequence du manuel, dans l'ordre.
        # LE COMPTE, ET PAS SEULEMENT LA PRESENCE.
        #
        # La sequence du manuel vide le cache DEUX fois -- avant l'ecriture, pour
        # que rien n'y survive sous l'ancien type, et apres, pour que rien n'y
        # entre sous le nouveau avant que le TLB ne soit refait. Meme chose pour
        # le TLB. Une premiere version de cette regle cherchait la PRESENCE de
        # `wbinvd` : en retirer un sur deux la laissait verte.
        for element, minimum, pourquoi in (
            ("wbinvd", 2, "le cache se vide de part et d'autre de l'ecriture ; des "
                          "lignes ecrites sous l'ancien type survivraient sinon "
                          "sous le nouveau"),
            ("cr0", 2, "le cache est desactive pendant la sequence, puis restaure"),
            ("cr3", 3, "le TLB se vide de part et d'autre de l'ecriture"),
            ("wrmsr", 1, "la PAT s'ecrit par MSR"),
            ("cli", 1, "la sequence ne tolere pas d'etre interrompue"),
        ):
            if pat.count(element) < minimum:
                fautes.append(
                    "pat.rs : `%s` apparait %d fois, il en faut %d -- %s."
                    % (element, pat.count(element), minimum, pourquoi)
                )
        # Les quatre premieres entrees ne se touchent pas.
        m = re.search(r"PAT_DEFAUT & !\(0xFFu64 << (\d+)\)", pat)
        if m is None:
            fautes.append(
                "pat.rs : la nouvelle PAT n'est plus derivee de la valeur par "
                "defaut. Toutes les pages du systeme designent les entrees zero a "
                "trois : les changer changerait le sens de tout d'un coup."
            )
        elif int(m.group(1)) < 32:
            fautes.append(
                "pat.rs : l'entree reprogrammee est en dessous de l'indice quatre. "
                "Les indices zero a trois sont ceux que designent les pages "
                "existantes -- une page sans PAT, PCD ni PWT designe zero."
            )
        if "entree_quatre_est_combinee" not in pat:
            fautes.append(
                "pat.rs : le resultat de l'ecriture n'est plus RELU. Un `wrmsr` "
                "accepte sans effet -- sous un hyperviseur qui filtre -- "
                "laisserait croire au correctif sans le rendre."
            )

    # La configuration doit etre sur le chemin commun ET sur les coeurs secondaires.
    principal = lit(MAIN)
    if principal is None or "pat::configure_ce_coeur()" not in principal:
        fautes.append(
            "main.rs : la PAT n'est plus configuree sur le chemin commun. Le "
            "chemin UEFI du bureau n'atteint jamais `arch::x86_64::init()` -- "
            "`stage2::run` est appele avant et ne rend pas la main."
        )
    smp = lit(SMP)
    if smp is None or "pat::configure_ce_coeur()" not in smp:
        fautes.append(
            "smp.rs : les coeurs secondaires ne configurent plus leur PAT. Elle "
            "est PAR COEUR : un coeur oublie rend une trame cent fois plus lente, "
            "sans fauter et sans le dire."
        )

    # La bascule des pages.
    vmm = lit(VMM)
    if vmm is None or "fn passe_en_ecriture_combinee" not in vmm:
        fautes.append("virtual.rs : la bascule des pages en ecriture combinee a disparu.")
    else:
        corps = vmm[vmm.find("fn passe_en_ecriture_combinee"):]
        corps = corps[:corps.find("\npub fn ", 10)] if "\npub fn " in corps[10:] else corps[:8000]
        if "2 * 1024 * 1024" not in corps:
            fautes.append(
                "virtual.rs : la bascule ne refuse plus une page d'un gibioctet. "
                "Une telle page couvre bien plus que le framebuffer : la passer "
                "en ecriture combinee emporterait des registres de peripherique "
                "voisins, dont l'ORDRE des ecritures est le seul contrat."
            )
        if "PTE_PAT_GRANDE_PAGE" not in corps or "PTE_PAT_PETITE_PAGE" not in corps:
            fautes.append(
                "virtual.rs : la bascule confond les deux positions du bit PAT. "
                "Il est au septieme bit sur une page de quatre kibioctets et au "
                "douzieme sur une grande page ; s'y tromper ne se remarque qu'a "
                "l'execution."
            )

    # La barriere apres chaque presentation.
    ecran = lit(ECRAN)
    if ecran is None:
        fautes.append("bochs.rs : absent.")
    else:
        if ecran.count("sfence") < 2:
            fautes.append(
                "bochs.rs : une presentation n'est plus suivie d'une barriere. En "
                "ecriture combinee les ecritures restent dans les tampons du "
                "processeur : sans `sfence`, la trame n'est pas a l'ecran quand on "
                "croit l'y avoir mise -- et la relecture de verification lirait "
                "des pixels perimes, ce qui eteindrait le chemin rapide pour "
                "toujours."
            )
        if "passe_en_ecriture_combinee" not in ecran:
            fautes.append(
                "bochs.rs : le framebuffer du micrologiciel n'est plus bascule en "
                "ecriture combinee apres son installation."
            )
        if "debit_framebuffer_mio_s" not in ecran:
            fautes.append(
                "bochs.rs : le debit vers le framebuffer n'est plus mesure. C'est "
                "le seul chiffre qui distingue un framebuffer combinable d'un "
                "framebuffer non cachable, et il a fallu un enregistreur de vol "
                "pour l'obtenir la premiere fois."
            )

    if fautes:
        print("ecriture combinee : %d probleme(s)\n" % len(fautes))
        for f in fautes:
            print("  - %s\n" % f)
        return 1
    print(
        "ecriture combinee : sequence PAT complete, entrees zero a trois "
        "intactes, resultat relu, configuration sur le chemin commun et sur "
        "chaque coeur, grande page refusee, barriere apres presentation, debit "
        "mesure"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
