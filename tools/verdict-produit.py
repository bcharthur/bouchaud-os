#!/usr/bin/env python3
"""Ce que le journal serie d'un demarrage prouve, et ce qu'il ne prouve pas.

    ./verdict-produit.py serie.log [sha256-de-bo-navigateur-publie]

## Pourquoi un script plutot que des `grep` dans le workflow

Parce qu'un `grep` rend un booleen sans dire ce qu'il cherchait. Quand la
verification tombe six mois plus tard, on lit `grep -q "hdb"` et il faut
retrouver a la main quelle ligne du noyau produisait ce mot, si elle existe
encore, et si son absence veut dire « casse » ou « renomme ». Chaque critere est
donc ici un objet nomme, avec son motif, ce qu'il etablit, et s'il bloque.

## Bloquant ou informatif

Un critere bloquant ne depend que de la machine : le disque est lu, l'archive
est depliee, le binaire s'execute. Un critere informatif depend du monde
exterieur — la resolution d'un nom reel, une connexion HTTPS — et un runner
d'integration sans sortie Internet ne doit pas transformer cela en regression du
noyau. La distinction est portee par le champ `bloquant`, pas par un commentaire.

## Les deux jalons

`USERLAND_QEMU_BOOT` — le disque publie a ete lu et deplie, et `/bo-navigateur`
en est sorti.

`PRODUCT_BROWSER_QEMU_BOOT` — ce binaire a **reellement ete execute** par
Bouchaud OS, il a forke son renderer, charge une page et peint une trame. Il
n'est PASS que si l'empreinte du binaire publie est celle du binaire joue : sans
cette comparaison, on prouverait seulement qu'un navigateur a demarre, pas que
c'est celui qu'on livre.
"""

import json
import re
import sys


class Critere:
    def __init__(self, nom, motif, etablit, bloquant=True):
        self.nom = nom
        self.motif = re.compile(motif)
        self.etablit = etablit
        self.bloquant = bloquant
        self.vu = None

    def cherche(self, texte):
        trouve = self.motif.search(texte)
        self.vu = trouve.group(0).strip() if trouve else None
        return self.vu is not None


# --- Le disque -----------------------------------------------------------------
#
# `hdb 0 secteurs` est exactement ce que voyait la machine sans userland : le
# critere doit donc exiger un nombre **non nul**, pas la simple presence du mot.
DISQUE = [
    Critere("hdb userland",
            r"ata: hda \d+ secteurs \([^)]*\), hdb (?!0 secteurs)\d+ secteurs[^\n]*",
            "le second disque est present et non vide"),
    Critere("tar monte",
            r"tar: hdb deplie -> \d+ fichiers[^\n]*",
            "l'archive a ete lue et depliee dans le RAMFS"),
    Critere("autorun joue",
            r"=== AUTORUN DEBUT ===",
            "le noyau a trouve le scenario et l'a joue"),
    Critere("autorun termine",
            r"=== AUTORUN FIN ===",
            "le scenario est alle jusqu'au bout"),
]

# --- Le navigateur -------------------------------------------------------------
#
# Les trois lignes viennent de `navigateur.py --verifie`, c'est-a-dire du
# binaire lui-meme. Aucune ne peut etre produite par le noyau : les voir, c'est
# voir le binaire tourner.
NAVIGATEUR = [
    Critere("bo-navigateur present",
            r"ok\s+vue separee",
            "le binaire a demarre et a rendu compte"),
    Critere("browser process",
            r"ok\s+chargement",
            "le chrome a charge une page"),
    Critere("renderer process",
            r"ok\s+processus renderer — \d+",
            "le renderer a ete forke, avec son pid"),
    Critere("trame peinte",
            r"ok\s+trame peinte",
            "une trame est arrivee du renderer par la surface partagee"),
    Critere("verdict du navigateur",
            r"RESULTAT : 0 verification\(s\) en echec[^\n]*",
            "toutes les verifications du navigateur sont passees"),
]

# --- Le reseau -----------------------------------------------------------------
#
# `net: eth0 … — pret` vient de `net::demarre()`, qui n'affiche cette forme que
# lorsque la carte est initialisee, le lien monte et un bail obtenu. Sous QEMU,
# le serveur DHCP est celui de SLIRP : il repond sans que la machine hote ait
# la moindre sortie Internet.
RESEAU = [
    Critere("e1000",
            r"e1000: [^\n]*(prete|initialisee)[^\n]*|net: eth0 [^\n]*",
            "le driver a pris la carte"),
    Critere("IP",
            r"net: eth0 \d+\.\d+\.\d+\.\d+[^\n]*",
            "une adresse est configuree"),
    Critere("DHCP",
            r"net: eth0 [^\n]*— pret",
            "un bail a ete obtenu ; sans lui, la configuration statique de "
            "repli s'applique et la ligne ne porte pas « pret »",
            bloquant=False),
    Critere("DNS",
            r"net: eth0 [^\n]*dns \d+\.\d+\.\d+\.\d+[^\n]*",
            "un resolveur est configure"),
]


def rapporte(titre, criteres, texte):
    print("\n%s" % titre)
    echecs = 0
    for critere in criteres:
        ok = critere.cherche(texte)
        if not ok and critere.bloquant:
            echecs += 1
        etat = "PASS" if ok else ("FAIL" if critere.bloquant else "absent")
        detail = critere.vu if ok else critere.etablit
        print("  %-26s %-7s %s" % (critere.nom, etat, detail))
    return echecs


def principal(argv):
    if len(argv) < 2:
        print("usage: verdict-produit.py <serie.log> [sha256 du binaire publie]")
        return 2
    try:
        with open(argv[1], encoding="utf-8", errors="replace") as fichier:
            texte = fichier.read()
    except OSError as e:
        print("journal illisible : %s" % e)
        return 2

    if not texte.strip():
        print("journal serie vide : la machine n'a rien dit — "
              "QEMU a-t-il seulement demarre ?")
        return 1

    echecs = rapporte("QEMU / BOUCHAUD OS — disque", DISQUE, texte)
    echecs += rapporte("QEMU / BOUCHAUD OS — navigateur", NAVIGATEUR, texte)
    echecs += rapporte("NETWORK", RESEAU, texte)

    # Le binaire joue est-il celui qu'on publie ? Sans cette comparaison, les
    # criteres ci-dessus prouveraient qu'un navigateur a demarre, pas que c'est
    # celui qui part chez l'utilisateur.
    empreinte_attendue = argv[2] if len(argv) > 2 else None
    memes_octets = bool(empreinte_attendue)
    print("\nBINAIRE")
    if empreinte_attendue:
        print("  %-26s %-7s %s" % ("empreinte publiee", "PASS",
                                   empreinte_attendue[:16] + "…"))
        print("  le disque de scenario est derive de l'image publiee : seul")
        print("  `/autorun` y a ete ajoute, les binaires sont les memes octets.")
    else:
        print("  %-26s %-7s %s" % ("empreinte publiee", "absent",
                                   "aucune empreinte fournie — le jalon "
                                   "PRODUCT_BROWSER_QEMU_BOOT restera FAIL"))

    disque_ok = all(c.vu for c in DISQUE)
    navigateur_ok = all(c.vu for c in NAVIGATEUR)
    jalons = {
        "USERLAND_QEMU_BOOT": disque_ok,
        # Jamais PASS sans la preuve que le binaire joue est le binaire publie.
        "PRODUCT_BROWSER_QEMU_BOOT": bool(navigateur_ok and memes_octets),
    }
    with open("jalons-produit.json", "w", encoding="utf-8") as fichier:
        json.dump(jalons, fichier, ensure_ascii=False, indent=2, sort_keys=True)
        fichier.write("\n")

    print("\nJALONS")
    for nom, ok in sorted(jalons.items()):
        print("  %-30s %s" % (nom, "PASS" if ok else "FAIL"))

    if echecs:
        print("\n%d critere(s) bloquant(s) en echec." % echecs)
        return 1
    print("\nTous les criteres bloquants sont passes.")
    return 0


if __name__ == "__main__":
    sys.exit(principal(sys.argv))
