#!/usr/bin/env python3
"""LA CHRONOLOGIE DU DEMARRAGE A FROID, TOUS PROCESSUS CONFONDUS.

# Ce qu'il manquait

Les mesures du demarrage existaient, eparpillees dans trois vocabulaires qui
ne se recoupaient pas :

    hote      BROWSER_HOST_START, ..._INITIALIZED, M11_GUI_HANDSHAKE_OK, ...
    worker    WORKER_ETAPE t=... etape=...
    noyau     PERF_FORK, PERF_EXECVE, [PERF-PROC]

On pouvait lire chacune et ne rien conclure : le seul chiffre qui interesse
est une SOUSTRACTION entre deux d'entre elles. « Les 23,5 s entre
`processus_lance` et `main` sont-elles de l'ELF, des tables de pages ou des
fautes fichier ? » demande les trois lignes a la fois.

# Ce que ce programme fait, et ce qu'il ne fait pas

Il RASSEMBLE et il SOUSTRAIT. Il ne juge pas : aucun budget, aucun verdict.
Les verdicts sont ailleurs, et un outil qui melange lecture et jugement
oblige a le modifier chaque fois que l'un des deux change.

    ./tools/ci/analyse-demarrage.py serie-browser-host.log
"""
import re
import sys


def lignes(chemin):
    with open(chemin, "rb") as fichier:
        brut = fichier.read().decode("utf-8", "replace")
    # Les horodatages de console et les couleurs ANSI precedent chaque ligne.
    propre = re.sub(r"\x1b\[[0-9;]*m", "", brut)
    return propre.splitlines()


def millisecondes(texte):
    try:
        return int(texte)
    except (TypeError, ValueError):
        return None


def chronologie(source):
    """Les etapes horodatees, dans l'ordre de l'horloge et non du journal.

    Les trois processus ecrivent sur la MEME console serie : l'ordre des
    lignes est celui de l'ecriture, pas celui des instants. Trier sur `t=`
    est donc la seule facon d'obtenir une chronologie -- et c'est pour cela
    que l'horloge devait etre monotone ET absolue.
    """
    # LES LIGNES DU NOYAU ENTRENT DANS LA MEME CHRONOLOGIE, ET C'EST LICITE.
    #
    # `MonotonicTime::now()` de Ladybird appelle `clock_gettime(CLOCK_MONOTONIC)`,
    # que ce noyau sert depuis `timer::monotonic_ns()`. Les deux `t=` sont donc
    # la MEME horloge, et les soustraire a un sens -- c'est exactement ce qu'il
    # faut pour repondre a « les 23,5 s avant `main` sont combien d'ELF, combien
    # de fautes fichier ».
    motifs = [
        (r"WORKER_ETAPE t=(\d+) (.*)$", "worker"),
        (r"SPAWN_ETAPE t=(\d+) (.*)$", "spawn"),
        (r"PERF_FORK t=(\d+) (.*)$", "noyau"),
        (r"PERF_EXECVE t=(\d+) (.*)$", "noyau"),
        (r"\[PERF-PROC\] t=(\d+) (.*)$", "proc"),
    ]
    etapes = []
    for ligne in source:
        for motif, origine in motifs:
            m = re.search(motif, ligne)
            if m:
                etapes.append((int(m.group(1)), origine, m.group(2).strip()))
                break
    etapes.sort(key=lambda e: e[0])
    return etapes


def jalons_hote(source):
    interessants = [
        "BROWSER_HOST_START",
        "BROWSER_HOST_INITIALIZED",
        "M11_GUI_HANDSHAKE_OK",
        "BROWSER_HOST_M11_FRAME_PRESENTED",
        "M11_DOCUMENT_LOADED",
    ]
    vus = {}
    for numero, ligne in enumerate(source):
        for jalon in interessants:
            if jalon in ligne and jalon not in vus:
                vus[jalon] = numero
    return [(jalon, vus[jalon]) for jalon in interessants if jalon in vus]


def noyau(source):
    forks, execs, procs = [], [], []
    for ligne in source:
        if "PERF_FORK " in ligne:
            forks.append(ligne[ligne.index("PERF_FORK "):].strip())
        elif "PERF_EXECVE " in ligne:
            execs.append(ligne[ligne.index("PERF_EXECVE "):].strip())
        elif "[PERF-PROC]" in ligne:
            procs.append(ligne[ligne.index("[PERF-PROC]"):].strip())
    return forks, execs, procs


def dernier_par_pid(lignes_proc):
    """Le dernier releve de chaque pid : c'est celui qui porte le total."""
    par_pid = {}
    for ligne in lignes_proc:
        m = re.search(r"pid=(\d+)", ligne)
        if m:
            par_pid[int(m.group(1))] = ligne
    return [par_pid[pid] for pid in sorted(par_pid)]


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    source = lignes(sys.argv[1])

    print("== jalons de l'hote, dans l'ordre du journal ==")
    for jalon, _ in jalons_hote(source):
        print(f"   {jalon}")
    if not jalons_hote(source):
        print("   (aucun)")

    etapes = chronologie(source)
    print()
    print("== chronologie commune : page, hote, worker et noyau ==")
    if not etapes:
        print("   (aucune etape horodatee)")
    else:
        origine = etapes[0][0]
        precedent = origine
        for t, source_etape, texte in etapes:
            print(f"   T+{(t - origine) / 1000:8.3f}s  (+{(t - precedent) / 1000:7.3f}s)"
                  f"  [{source_etape}] {texte}")
            precedent = t

    forks, execs, procs = noyau(source)
    print()
    print("== ce que le noyau a mesure : duplication d'espace ==")
    for ligne in forks[:12] or ["   (aucune)"]:
        print(f"   {ligne}")
    print()
    print("== ce que le noyau a mesure : etapes de l'execve ==")
    for ligne in execs[:12] or ["   (aucune)"]:
        print(f"   {ligne}")
    print()
    print("== fautes de page par processus (dernier releve de chaque pid) ==")
    for ligne in dernier_par_pid(procs) or ["   (aucune)"]:
        print(f"   {ligne}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
