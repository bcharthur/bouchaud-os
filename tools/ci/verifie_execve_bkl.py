#!/usr/bin/env python3
"""Garde-fou des chemins noyau QUI NE REVIENNENT PAS.

BOUCHAUD_C39_PORTEES_ABANDONNEES

`syscall_dispatch` ouvre deux gardes RAII avant d'executer un appel systeme :

    let _domaine = sync::portee(sync::Domaine::Syscall);
    let kernel   = smp_lock::enter();

Deux chemins n'en ressortent jamais -- `execve` reussi, qui saute en ring 3, et
le retrait d'un zombie, qui commute sans retour. Les deux abandonnent la pile
noyau, donc les deux `Drop`.

Le gros verrou etait deja compense sur les deux chemins. La portee de domaine
ne l'etait sur aucun : `sommet[cpu]` montait d'un cran par `execve` et d'un
autre par `exit`, sans jamais redescendre. Au-dela de `PROFONDEUR = 8`,
`courant()` rend une case figee et TOUTE prise du gros verrou est attribuee a
un domaine mort -- l'instrument de sortie du gros verrou se detruit lui-meme.

Ce script relit `PERF_EXECVE_BKL` et refuse de conclure sans mesure.
"""
import re
import sys

LIGNE = re.compile(
    r"PERF_EXECVE_BKL t=(?P<t>\d+) pid=(?P<pid>\d+) cpu=(?P<cpu>\d+)"
    r" depth_avant=(?P<depth_avant>\d+) depth_abandonne=(?P<depth_abandonne>\d+)"
    r" owner_apres=(?P<owner_apres>-?\d+)"
    r" domaines_avant=(?P<domaines_avant>\d+) domaines_apres=(?P<domaines_apres>\d+)"
    r" portees_refermees=(?P<portees_refermees>\d+)"
    r" debordements=(?P<debordements>\d+) reperes_perimes=(?P<reperes_perimes>\d+)"
)

# Trois `execve` suffisent a voir une fuite qui avance d'un cran a chaque
# passage, et c'est ce que produit le banc du cout du fork.
MINIMUM = 3


def erreur(message, indice=None):
    print(f"execve/bkl : {message}", file=sys.stderr)
    if indice:
        print(f"             {indice}", file=sys.stderr)
    return 1


def main():
    if len(sys.argv) < 2:
        return erreur("usage: verifie_execve_bkl.py <journal serie>")
    chemin = sys.argv[1]
    try:
        with open(chemin, "rb") as flux:
            texte = flux.read().decode("utf-8", "replace")
    except OSError as exc:
        return erreur(f"journal illisible : {exc}")

    lignes = [m.groupdict() for m in LIGNE.finditer(texte)]

    # --- fail-closed : sans mesure, pas de vert -----------------------------
    brutes = texte.count("PERF_EXECVE_BKL")
    if brutes and len(lignes) != brutes:
        return erreur(
            f"{brutes - len(lignes)} ligne(s) PERF_EXECVE_BKL non analysables",
            "le format de la sonde a change sans que ce script suive",
        )
    if len(lignes) < MINIMUM:
        return erreur(
            f"{len(lignes)} releve(s) PERF_EXECVE_BKL, il en faut {MINIMUM}",
            "le scenario n'a pas execve assez de fois : rien n'a ete verifie",
        )

    echecs = []

    # --- 1. aucun proprietaire fantome --------------------------------------
    #
    # Le CPU qui saute en ring 3 ne doit plus detenir le gros verrou. S'il
    # figure encore comme proprietaire, le nouveau programme tourne en anneau 3
    # pendant que toute la machine attend un verrou que personne ne rendra.
    for l in lignes:
        if int(l["owner_apres"]) == int(l["cpu"]):
            echecs.append(
                f"pid={l['pid']} : le CPU {l['cpu']} detient encore le gros verrou"
                f" apres la bascule en ring 3 (proprietaire fantome)"
            )

    # --- 2. le chemin passait bien avec le verrou ---------------------------
    #
    # Si `depth_abandonne` valait zero, la sonde ne prouverait rien : elle
    # observerait un chemin qui n'avait rien a rendre.
    for l in lignes:
        if int(l["depth_abandonne"]) < 1:
            echecs.append(
                f"pid={l['pid']} : depth_abandonne=0, ce chemin ne detenait pas"
                f" le gros verrou -- la mesure ne demontre rien"
            )

    # --- 3. coherence interne de la sonde -----------------------------------
    for l in lignes:
        attendu = int(l["domaines_avant"]) - int(l["portees_refermees"])
        if int(l["domaines_apres"]) != attendu:
            echecs.append(
                f"pid={l['pid']} : domaines_apres={l['domaines_apres']} alors que"
                f" {l['domaines_avant']} - {l['portees_refermees']} = {attendu}"
            )

    # --- 4. LA fuite : la profondeur de portees ne doit pas croitre ----------
    #
    # C'est l'assertion qui rougit si la compensation disparait. Sous le defaut,
    # `domaines_avant` avance de deux a chaque enfant (un pour l'`execve`, un
    # pour l'`exit`) : 13, 15, 17...
    profondeurs = [int(l["domaines_avant"]) for l in lignes]
    if max(profondeurs) > min(profondeurs):
        echecs.append(
            f"la profondeur de portees CROIT : {profondeurs}"
            f" -- une pile noyau abandonnee ne referme pas sa PorteeDomaine"
        )

    # --- 5. jamais de debordement -------------------------------------------
    #
    # `PROFONDEUR` vaut 8. Un debordement signifie que l'attribution des prises
    # du gros verrou est deja fausse, et le restera.
    debordements = max(int(l["debordements"]) for l in lignes)
    if debordements:
        echecs.append(
            f"debordements={debordements} : la pile de domaines a depasse sa"
            f" profondeur, l'attribution du gros verrou est corrompue"
        )

    if echecs:
        print("execve/bkl : chemins no-return NON conformes", file=sys.stderr)
        for e in echecs:
            print(f"  - {e}", file=sys.stderr)
        return 1

    perimes = max(int(l["reperes_perimes"]) for l in lignes)
    print(
        f"execve/bkl : {len(lignes)} releves, profondeur stable a"
        f" {profondeurs[0]}, aucun proprietaire fantome,"
        f" debordements=0, reperes_perimes={perimes}"
    )
    print("EXECVE_BKL_OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
