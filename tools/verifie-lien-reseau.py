#!/usr/bin/env python3
"""Garde-fou : l'etat du lien se VEILLE, il ne se constate pas une fois.

# Le defaut, lu sur la machine de reference

Le releve du 12 septembre 2026 contient ces trois lignes, et rien apres :

    BOUCHAUD_TRIGKEY_RTL8168_DRIVER_OK mac=b0:41:6f:09:70:a1
    BOUCHAUD_TRIGKEY_RTL8168_LINK_DOWN
    net: lo actif ; eth0 initialisee, lien bas

La carte est reconnue et le pilote fonctionne. Le lien est bas AU MOMENT OU ON
REGARDE -- cinq secondes apres la mise sous tension, ce qui est court pour une
autonegociation cuivre gigabit et bien plus court que le temps de brancher un
cable. Apres quoi plus personne ne regardait : `demarre()` etait appele une
fois, son verdict etait definitif, et la machine restait hors ligne pour le
reste de la session.

Cote utilisateur, cela donne un navigateur qui repond « Unable to resolve
host » a toutes les pages, sur une machine dont la carte reseau marche.

# Ce qui est verifie

1. Un fil relit l'etat du lien a cadence bornee.
2. Il retente la configuration quand le lien MONTE.
3. Il redescend le verdict quand le lien TOMBE -- une configuration qui ne mene
   plus nulle part fait attendre chaque requete jusqu'a son echeance.
4. Il ne FABRIQUE aucune adresse : les 10.0.2.x sont une convention QEMU, et
   les inventer sur du materiel reel ferait passer « hors ligne » pour
   « configure ».
5. Il dort entre deux lectures.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
NET = RACINE / "src/net/mod.rs"
STAGE2 = RACINE / "src/platform/pc/stage2.rs"
MAIN = RACINE / "src/main.rs"


def sans_commentaires(texte):
    return "\n".join(
        l for l in texte.splitlines() if not l.lstrip().startswith("//")
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
    fautes = []
    for chemin in (NET, STAGE2, MAIN):
        if not chemin.exists():
            print("  - fichier absent : %s" % chemin)
            return 1

    net = sans_commentaires(NET.read_text(encoding="utf-8"))
    veilleur = corps(net, "fn veilleur_de_lien()")
    if veilleur is None:
        fautes.append(
            "net/mod.rs : le veilleur de lien a disparu. Un cable branche "
            "apres le demarrage ne sera plus jamais vu, et la machine restera "
            "hors ligne pour toute la session."
        )
    else:
        if "link_up()" not in veilleur:
            fautes.append(
                "net/mod.rs : le veilleur ne relit plus l'etat du lien."
            )
        if "sleep_ticks" not in veilleur:
            fautes.append(
                "net/mod.rs : le veilleur ne dort plus entre deux lectures ; "
                "il brulerait un coeur pour lire un registre."
            )
        if "dhcp::negocie()" not in veilleur:
            fautes.append(
                "net/mod.rs : le veilleur ne retente plus la configuration "
                "quand le lien monte. Voir le lien monter sans rien en faire "
                "ne sert a rien."
            )
        if "Demarrage::LienBas" not in veilleur:
            fautes.append(
                "net/mod.rs : le veilleur ne redescend plus le verdict quand "
                "le lien tombe. Chaque requete partirait alors dans le vide et "
                "attendrait son echeance."
            )
        # La regle qui compte : ne JAMAIS fabriquer une configuration QEMU sur
        # du materiel reel. `SansBail` n'est legitime que pour SLIRP.
        if "using_rtl8168()" not in veilleur:
            fautes.append(
                "net/mod.rs : le veilleur ne distingue plus la carte physique "
                "de SLIRP. Sur RTL8168, un DHCP absent deviendrait la fausse "
                "configuration 10.0.2.x, et « hors ligne » passerait pour "
                "« configure »."
            )

    m = re.search(r"const PERIODE_LIEN_MS: u64 = ([0-9_]+);", net)
    if m is None:
        fautes.append("net/mod.rs : la periode de relecture du lien n'est plus lisible.")
    elif int(m.group(1).replace("_", "")) > 5_000:
        fautes.append(
            "net/mod.rs : le lien est relu moins d'une fois toutes les cinq "
            "secondes. Un cable branche se verrait avec un retard sensible."
        )

    if "PLAFOND_DHCP_MS" not in net or "saturating_mul(2)" not in net:
        fautes.append(
            "net/mod.rs : l'attente entre deux reprises DHCP n'augmente plus "
            "apres un echec. Un reseau cable sans serveur DHCP est une "
            "situation durable : la retenter toutes les dix secondes pendant "
            "des heures est du bruit."
        )

    m = re.search(r"const PERIODE_DHCP_MS: u64 = ([0-9_]+);", net)
    if m is None:
        fautes.append("net/mod.rs : la periode de reprise DHCP n'est plus lisible.")
    elif int(m.group(1).replace("_", "")) < 2_000:
        fautes.append(
            "net/mod.rs : les tentatives DHCP s'enchainent sans pause. "
            "`negocie()` attend lui-meme plusieurs secondes : les relancer "
            "sans repit tiendrait le reseau occupe en permanence pour un "
            "serveur qui, le plus souvent, n'existe pas."
        )

    for chemin, nom in ((STAGE2, "stage2.rs"), (MAIN, "main.rs")):
        texte = sans_commentaires(chemin.read_text(encoding="utf-8"))
        if "demarre_le_veilleur_de_lien()" not in texte:
            fautes.append(
                "%s : le veilleur de lien n'est plus lance sur ce chemin de "
                "demarrage." % nom
            )

    if fautes:
        print("lien reseau : %d probleme(s)\n" % len(fautes))
        for f in fautes:
            print("  - %s\n" % f)
        return 1
    print(
        "lien reseau : veille bornee, reprise DHCP a la montee, verdict "
        "redescendu a la chute, aucune adresse QEMU fabriquee sur materiel "
        "reel, veilleur lance sur les deux chemins de demarrage"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
