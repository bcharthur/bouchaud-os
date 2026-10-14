#!/usr/bin/env python3
"""Mesure la PROFONDEUR DE PILE du noyau, par analyse du graphe d'appels.

# Ce qu'aucun test ne voit

Une pile noyau qui deborde n'a pas de symptome propre : elle ecrit dans ce qui
la precede en memoire, silencieusement, et la faute apparait ailleurs -- ou
nulle part, jusqu'a ce que `RSP` sorte de toute region valide et que le
processeur double-faute. Le double fault ne nomme alors ni la fonction fautive
ni la profondeur atteinte : il nomme la DERNIERE instruction, qui est
innocente.

Le seul moment ou ce defaut est visible est la COMPILATION. Chaque fonction
declare sa trame dans son prologue (`sub $N, %rsp`), et la somme le long d'une
chaine d'appels est la profondeur que cette chaine consomme. C'est ce que ce
programme calcule.

# Pourquoi la profondeur, et pas la plus grosse trame

Une trame de 8 Kio n'est pas un defaut si elle est appelee depuis la racine.
Une trame de 512 octets l'est si elle est au fond d'une chaine de cent
fonctions. La question n'est jamais « quelle est la plus grosse trame » mais
« combien la pire chaine consomme-t-elle », et la reponse demande le graphe.

# Les limites, dites plutot que tues

  * Les appels INDIRECTS (`call *%rax`) ne sont pas suivis : le graphe ne sait
    pas quelle fonction un pointeur designe. Les objets-traits du noyau --
    `PiloteBloc`, les gestionnaires -- passent par la, et leur profondeur est
    donc SOUS-ESTIMEE. Les points d'entree connus sont ajoutes a la main dans
    `RACINES_INDIRECTES` pour compenser.
  * Une allocation dynamique sur la pile (`alloca`, tableau de taille
    variable) n'apparait pas dans le prologue. Rust n'en produit pas.
  * La recursion est coupee : une chaine qui revient sur elle-meme est comptee
    une fois et signalee.

Ces limites font de ce chiffre une BORNE INFERIEURE du pire cas. Une borne
inferieure qui depasse deja la pile est une preuve ; une borne inferieure qui
tient n'est pas une garantie, et le canari de la page de garde reste la mesure
d'execution.
"""

import argparse
import re
import subprocess
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]

ENTETE = re.compile(r"^([0-9a-f]+) <(.+)>:$")
SUB_RSP = re.compile(r"\bsub\s+\$0x([0-9a-f]+),%rsp\b")
PUSH = re.compile(r"\bpush\s+%r")
CALL = re.compile(r"\bcall\s+[0-9a-f]+ <([^>+]+)(?:\+0x[0-9a-f]+)?>")


def desassemble(binaire: Path) -> str:
    for outil in ("objdump", "llvm-objdump"):
        try:
            return subprocess.run(
                [outil, "-d", "--no-show-raw-insn", str(binaire)],
                capture_output=True,
                text=True,
                check=True,
            ).stdout
        except (FileNotFoundError, subprocess.CalledProcessError):
            continue
    raise SystemExit("ni objdump ni llvm-objdump n'est disponible")


def analyse(texte: str):
    """Rend (trames, appels) : taille de trame et appelles directs par fonction."""
    trames = {}
    appels = {}
    courant = None
    pousses = 0
    prologue = True
    for ligne in texte.splitlines():
        m = ENTETE.match(ligne.strip())
        if m:
            courant = m.group(2)
            trames.setdefault(courant, 0)
            appels.setdefault(courant, set())
            pousses = 0
            prologue = True
            continue
        if courant is None:
            continue
        if prologue and PUSH.search(ligne):
            pousses += 8
        m = SUB_RSP.search(ligne)
        if m and prologue:
            trames[courant] = int(m.group(1), 16) + pousses
            prologue = False
        m = CALL.search(ligne)
        if m:
            prologue = False
            appels[courant].add(m.group(1))
            # L'adresse de retour compte : elle est poussee par `call`.
    for nom in trames:
        if trames[nom] == 0:
            trames[nom] = pousses if False else trames[nom]
    return trames, appels


# BOUCHAUD_PILE_PROFONDEUR_DETERMINISTE_V1
#
# La memoisation de `profondeur_max` retient le resultat d'un noeud des que sa
# PIRE chaine n'est pas coupee par une recursion -- meme si une autre branche
# l'a ete. Ce resultat depend donc du chemin d'arrivee, c'est-a-dire de l'ordre
# de parcours. Cet ordre etait celui d'un `set` : il changeait avec
# PYTHONHASHSEED, et le garde aussi -- sur un meme binaire, 2 graines vertes
# sur 8, la chaine `window_manager::boucle` mesuree a 61 288 ou 63 320 octets
# pour une pile de 61 440. Un garde-fou qui depend d'une graine aleatoire est
# un faux vert en attente.
#
# Le calcul exact (plus longue chaine sans repetition) est exponentiel : un
# seul cycle du noyau compte 95 fonctions (boite noire, panique, xHCI, serie).
# Chaque parcours rend une chaine REELLE du graphe d'appels, donc une borne
# INFERIEURE vraie. On en fait plusieurs, dans des ordres fixes, et on garde la
# pire : deterministe, et jamais moins severe que le meilleur des parcours.
ORDRES = 32


def ordres_de_parcours(appels):
    """Les ORDRES tables d'appels, chacune dans un ordre fixe et reproductible."""
    import random
    tables = []
    for k in range(ORDRES):
        hasard = random.Random(k)
        table = {}
        for nom in sorted(appels):
            fils = sorted(appels[nom])
            if k == 1:
                fils.reverse()
            elif k >= 2:
                hasard.shuffle(fils)
            table[nom] = fils
        tables.append(table)
    return tables


def profondeur_max(trames, appels, depart, vus=None, memo=None):
    """Profondeur de la pire chaine partant de `depart`, et cette chaine.

    `appels` doit etre une table a listes ORDONNEES (voir `ordres_de_parcours`)
    pour que le resultat soit reproductible.
    """
    if memo is None:
        memo = {}
    if vus is None:
        vus = set()
    if depart in vus:
        # Recursion : la chaine est coupee ici. La compter indefiniment
        # rendrait n'importe quel graphe infini.
        return 0, [depart + " (recursion)"]
    if depart in memo:
        return memo[depart]

    propre = trames.get(depart, 0) + 8  # +8 : l'adresse de retour
    vus = vus | {depart}
    pire = 0
    chaine = []
    for appele in appels.get(depart, ()):
        if appele == depart:
            continue
        sous, sous_chaine = profondeur_max(trames, appels, appele, vus, memo)
        if sous > pire:
            pire = sous
            chaine = sous_chaine
    total = propre + pire
    resultat = (total, [depart] + chaine)
    # La memoisation n'est valable que hors cycle : une chaine coupee par
    # `vus` donnerait un resultat dependant du chemin d'arrivee.
    if not any("(recursion)" in e for e in resultat[1]):
        memo[depart] = resultat
    return resultat


def court(nom: str) -> str:
    """Un nom mangle raccourci a ce qui se lit."""
    if len(nom) <= 110:
        return nom
    return nom[:107] + "..."


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("binaire", nargs="?",
                   default="target/x86_64-bouchaud_os_uefi/debug/bouchaud-os")
    p.add_argument("--budget", type=int, default=0,
                   help="octets utilisables d'une pile noyau ; 0 pour ne rien exiger")
    p.add_argument("--racine", action="append", default=[],
                   help="motif de symbole a traiter comme point d'entree")
    p.add_argument("--top", type=int, default=15)
    p.add_argument("--trames", type=int, default=0,
                   help="signale toute trame unique depassant ce nombre d'octets")
    args = p.parse_args()

    binaire = Path(args.binaire)
    if not binaire.is_file():
        print("binaire absent : %s" % binaire)
        print("construire d'abord :")
        print("  cargo build --target targets/x86_64-bouchaud_os_uefi.json \\")
        print("    --no-default-features --features 'uefi-boot,reference-bringup,reference-desktop'")
        return 1

    trames, appels = analyse(desassemble(binaire))

    fautes = []

    if args.trames:
        grosses = sorted(((t, n) for n, t in trames.items() if t > args.trames), reverse=True)
        if grosses:
            print("TRAMES UNIQUES AU-DELA DE %d OCTETS" % args.trames)
            for taille, nom in grosses[:args.top]:
                print("  %8d  %s" % (taille, court(nom)))
            print()
            for taille, nom in grosses:
                fautes.append(
                    "trame unique de %d octets : %s" % (taille, court(nom))
                )

    if args.racine:
        print("PROFONDEUR DES CHAINES DEPUIS LES POINTS D'ENTREE")
        racines = sorted({nom for motif in args.racine for nom in trames if motif in nom})
        pires = {}
        for table in ordres_de_parcours(appels):
            memo = {}
            for nom in racines:
                total, chaine = profondeur_max(trames, table, nom, None, memo)
                if nom not in pires or total > pires[nom][0]:
                    pires[nom] = (total, chaine)
        mesures = [(total, nom, chaine) for nom, (total, chaine) in pires.items()]
        mesures.sort(reverse=True)
        vus = set()
        for total, nom, chaine in mesures:
            if nom in vus:
                continue
            vus.add(nom)
            marque = ""
            if args.budget and total > args.budget:
                marque = "  <<< DEPASSE LA PILE"
                fautes.append(
                    "la chaine partant de %s consomme %d octets, la pile en offre %d"
                    % (court(nom), total, args.budget)
                )
            print("  %8d  %s%s" % (total, court(nom), marque))
            if marque:
                for etage in chaine[:20]:
                    print("             -> %s" % court(etage))
        print()

    if fautes:
        print("pile noyau : %d probleme(s)\n" % len(fautes))
        for faute in fautes[:20]:
            print("  - %s\n" % faute)
        return 1

    print("pile noyau : aucune trame ni chaine au-dela des budgets demandes")
    return 0


if __name__ == "__main__":
    sys.exit(main())
