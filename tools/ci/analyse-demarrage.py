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
    """Les quatre familles de lignes qui bornent un lancement de service.

    `PERF_EXECVE` est indexe par image, `[PERF-PROC]` par pid, `SPAWN_ETAPE`
    par enfant et `WORKER_ETAPE` par pid : c'est ce recoupement qui manquait
    pour mettre les six services sur la meme ligne.
    """
    forks, execs, procs, spawns, workers = [], [], [], [], []
    for ligne in source:
        if "PERF_FORK " in ligne:
            forks.append(ligne[ligne.index("PERF_FORK "):].strip())
        elif "PERF_EXECVE " in ligne:
            execs.append(ligne[ligne.index("PERF_EXECVE "):].strip())
        elif "[PERF-PROC]" in ligne:
            procs.append(ligne[ligne.index("[PERF-PROC]"):].strip())
        elif "SPAWN_ETAPE " in ligne:
            spawns.append(ligne[ligne.index("SPAWN_ETAPE "):].strip())
        elif "WORKER_ETAPE " in ligne:
            workers.append(ligne[ligne.index("WORKER_ETAPE "):].strip())
    return forks, execs, procs, spawns, workers


def series_par_pid(lignes_proc, maximum=8):
    """La SERIE de chaque pid, pas seulement son dernier releve.

    Le dernier releve porte le total, ce qui repond a « combien au bout du
    compte ». La question posee est autre : « pendant les vingt-trois secondes
    avant `main`, ou est passe le temps ». Il y faut la progression -- un
    processus qui prend dix mille fautes fichier dans ses cinq premieres
    secondes puis plus rien ne se soigne pas comme un qui faute regulierement.
    """
    par_pid = {}
    for ligne in lignes_proc:
        m = re.search(r"pid=(\d+)", ligne)
        if m:
            par_pid.setdefault(int(m.group(1)), []).append(ligne)
    sortie = []
    for pid in sorted(par_pid):
        serie = par_pid[pid]
        # Les premiers relevés portent le demarrage ; le dernier porte le
        # total. Entre les deux, on echantillonne plutot que de tout imprimer.
        if len(serie) <= maximum:
            retenus = serie
        else:
            pas = len(serie) / (maximum - 1)
            retenus = [serie[int(i * pas)] for i in range(maximum - 1)] + [serie[-1]]
        sortie.extend(retenus)
    return sortie


SERVICES = [
    "BouchaudBrowserHost",
    "RequestServer",
    "ImageDecoder",
    "Compositor",
    "WebContent",
    "WebWorker",
]

# UNE COLONNE ABSENTE N'EST PAS UNE COLONNE NULLE.
#
# BOUCHAUD_C42_INCONNU_N_EST_PAS_ZERO
#
# `0` est une mesure : ce chemin n'a rien coute. `-` est l'aveu qu'on n'a pas
# regarde. Les confondre fait disparaitre precisement le temps qu'on cherche --
# un poste non instrumente s'affiche a zero et sort du classement.
INCONNU = "-"


def _entier(ligne, motif):
    m = re.search(motif, ligne)
    return int(m.group(1)) if m else None


def _ms(v, diviseur=1000):
    if v is None:
        return INCONNU
    return f"{v / diviseur:.1f}"


def _n(v):
    return INCONNU if v is None else str(v)


def tableau_des_services(execs, forks, spawns, procs, workers):
    """Une entree par INSTANCE de service, avec ses bornes et ses fautes.

    Les sources sont indexees differemment -- `PERF_EXECVE` par image,
    `[PERF-PROC]` par pid, `SPAWN_ETAPE` par enfant, `WORKER_ETAPE` par pid --
    et c'est ce recoupement qui manquait pour comparer les six services.
    """
    instances = []
    for ligne in execs:
        image = re.search(r"image=(\S+)", ligne)
        pid = _entier(ligne, r"pid=(\d+)")
        if not image or pid is None:
            continue
        nom = image.group(1).rsplit("/", 1)[-1]
        if nom not in SERVICES:
            continue
        exec_exit = _entier(ligne, r"\bt=(\d+)")
        exec_us = _entier(ligne, r"duree_us=(\d+)")
        exec_enter = None
        if exec_exit is not None and exec_us is not None:
            exec_enter = exec_exit - exec_us // 1000
        instances.append({
            "service": nom,
            "pid": pid,
            "exec_enter_ms": exec_enter,
            "exec_exit_ms": exec_exit,
            "exec_duration_us": exec_us,
            # La liberation de l'ancien espace est DANS l'execve : la sortir
            # permet de voir qu'elle en fait l'essentiel.
            "exec_liberation_us": _entier(ligne, r"liberation_us=(\d+)"),
        })

    par_pid = {i["pid"]: i for i in instances}
    for i in instances:
        for cle in ("spawn_request_ms", "fork_enter_ms", "fork_exit_ms",
                    "fork_duration_ms", "fork_noyau_us", "fork_pages",
                    "first_user_instruction_ms", "main_ms", "ready_ms",
                    "rss_kio", "faults_total", "faults_total_us",
                    "worst_fault_us", "file_faults", "file_fault_us",
                    "zero_faults", "zero_fault_us", "shared_faults",
                    "shared_fault_us", "copy_faults", "copy_fault_us",
                    "wait_faults", "wait_fault_us"):
            i.setdefault(cle, None)

    # --- le fork vu de Ladybird (SPAWN_ETAPE, indexe par enfant) ------------
    for ligne in spawns:
        pid = _entier(ligne, r"enfant=(\d+)")
        i = par_pid.get(pid)
        if i is None:
            continue
        i["fork_exit_ms"] = _entier(ligne, r"\bt=(\d+)")
        i["fork_enter_ms"] = _entier(ligne, r"debut=(\d+)")
        i["fork_duration_ms"] = _entier(ligne, r"fork_ms=(\d+)")

    # --- le fork vu du noyau (PERF_FORK), pour recouper ---------------------
    for ligne in forks:
        pid = _entier(ligne, r"enfant=(\d+)")
        i = par_pid.get(pid)
        if i is None:
            continue
        i["fork_noyau_us"] = _entier(ligne, r"duree_us=(\d+)")
        i["fork_pages"] = _entier(ligne, r"pages_copiees=(\d+)")

    # --- les fautes (dernier releve de chaque pid : il porte les totaux) ----
    for ligne in procs:
        pid = _entier(ligne, r"pid=(\d+)")
        i = par_pid.get(pid)
        if i is None:
            continue
        i["rss_kio"] = _entier(ligne, r"rss_kio=(\d+)")
        i["faults_total"] = _entier(ligne, r"fautes=(\d+)")
        i["faults_total_us"] = _entier(ligne, r"total_us=(\d+)")
        i["worst_fault_us"] = _entier(ligne, r"pire_us=(\d+)")
        for cle, mot in (("zero", "zero"), ("file", "fichier"),
                         ("shared", "partage"), ("copy", "copie"),
                         ("wait", "attente")):
            m = re.search(rf"{mot}=(\d+)/(\d+)us", ligne)
            if m:
                i[f"{cle}_faults"] = int(m.group(1))
                i[f"{cle}_fault_us"] = int(m.group(2))

    # --- les bornes cote Ladybird (WORKER_ETAPE, worker seulement) ----------
    for ligne in workers:
        pid = _entier(ligne, r"pid=(\d+)")
        i = par_pid.get(pid)
        if i is None:
            continue
        t = _entier(ligne, r"\bt=(\d+)")
        if "etape=main" in ligne:
            i["main_ms"] = t
        elif "etape=boucle_prete" in ligne:
            i["ready_ms"] = t
        elif "etape=ipc_pret" in ligne and i["ready_ms"] is None:
            i["ready_ms"] = t

    instances.sort(key=lambda i: (SERVICES.index(i["service"]),
                                  i["exec_exit_ms"] or 0))
    return instances


def imprime_services(instances):
    """Une LIGNE par instance, greppable, avec toutes les colonnes."""
    if not instances:
        print("   (aucun service reconnu ; ni PERF_EXECVE ni [PERF-PROC] ne les nomment)")
        return
    for i in instances:
        print(
            "   SERVICE_TIMELINE"
            f" service={i['service']}"
            f" pid={i['pid']}"
            f" spawn_request_ms={_ms(i['spawn_request_ms'], 1)}"
            f" fork_enter_ms={_ms(i['fork_enter_ms'], 1)}"
            f" fork_exit_ms={_ms(i['fork_exit_ms'], 1)}"
            f" fork_duration_ms={_n(i['fork_duration_ms'])}"
            f" fork_noyau_us={_n(i['fork_noyau_us'])}"
            f" fork_pages={_n(i['fork_pages'])}"
            f" exec_enter_ms={_ms(i['exec_enter_ms'], 1)}"
            f" exec_exit_ms={_ms(i['exec_exit_ms'], 1)}"
            f" exec_duration_us={_n(i['exec_duration_us'])}"
            f" exec_liberation_us={_n(i['exec_liberation_us'])}"
            f" first_user_instruction_ms={_ms(i['first_user_instruction_ms'], 1)}"
            f" main_ms={_ms(i['main_ms'], 1)}"
            f" ready_ms={_ms(i['ready_ms'], 1)}"
            f" rss_MiB={_n(i['rss_kio'] // 1024 if i['rss_kio'] is not None else None)}"
            f" faults_total={_n(i['faults_total'])}"
            f" faults_total_ms={_ms(i['faults_total_us'])}"
            f" worst_fault_ms={_ms(i['worst_fault_us'])}"
            f" file_faults={_n(i['file_faults'])}"
            f" file_fault_ms={_ms(i['file_fault_us'])}"
            f" zero_faults={_n(i['zero_faults'])}"
            f" zero_fault_ms={_ms(i['zero_fault_us'])}"
            f" shared_faults={_n(i['shared_faults'])}"
            f" shared_fault_ms={_ms(i['shared_fault_us'])}"
            f" copy_faults={_n(i['copy_faults'])}"
            f" copy_fault_ms={_ms(i['copy_fault_us'])}"
            f" wait_faults={_n(i['wait_faults'])}"
            f" wait_fault_ms={_ms(i['wait_fault_us'])}"
        )
    print()
    print("   Colonnes a `-` : non mesurees, PAS nulles.")
    print("   spawn_request_ms          : pas de sonde a l'entree de Core::Process::spawn.")
    print("   first_user_instruction_ms : rien ne borne encore la premiere")
    print("                               instruction anneau 3 apres l'execve.")
    print("   main_ms / ready_ms        : instrumentes pour WebWorker seulement.")


def decompose(i):
    """Decompose le demarrage d'une instance, et AVOUE ce qui reste.

    BOUCHAUD_C43_LE_RESIDU_EST_LA_MESURE

    Les cinq segments sont contigus par construction : leur somme vaut le total
    par definition, et un « residu » calcule ainsi vaudrait toujours zero. Ce
    n'est pas la question posee.

    La question est : de ce total, quelle part a une CAUSE NOMMEE ? Seuls le
    `fork`, l'`execve` et les fautes de page en ont une aujourd'hui. Le reste
    est du temps qui passe sans qu'on sache ou -- et c'est ce chiffre-la qui
    dit ou instrumenter ensuite.
    """
    debut = i["fork_enter_ms"]
    fin = i["ready_ms"] or i["main_ms"] or i["exec_exit_ms"]
    if debut is None or fin is None:
        return None
    total = fin - debut

    segments = {
        "launch_to_fork_enter_ms": None,  # pas de sonde a l'entree de spawn
        "fork_ms": i["fork_duration_ms"],
        "fork_exit_to_exec_enter_ms": (
            i["exec_enter_ms"] - i["fork_exit_ms"]
            if i["exec_enter_ms"] is not None and i["fork_exit_ms"] is not None
            else None
        ),
        "exec_ms": (
            i["exec_duration_us"] // 1000
            if i["exec_duration_us"] is not None else None
        ),
        "exec_exit_to_main_ms": (
            i["main_ms"] - i["exec_exit_ms"]
            if i["main_ms"] is not None and i["exec_exit_ms"] is not None
            else None
        ),
        "main_to_ready_ms": (
            i["ready_ms"] - i["main_ms"]
            if i["ready_ms"] is not None and i["main_ms"] is not None
            else None
        ),
    }

    # Les fautes n'ajoutent pas de temps : elles EXPLIQUENT du temps deja
    # compte dans les segments ci-dessus. Les additionner au total serait le
    # compter deux fois.
    causes = {
        "page_fault_file_ms": i["file_fault_us"],
        "page_fault_zero_ms": i["zero_fault_us"],
        "page_fault_wait_ms": i["wait_fault_us"],
        "page_fault_copy_ms": i["copy_fault_us"],
        "page_fault_shared_ms": i["shared_fault_us"],
    }
    explique = (segments["fork_ms"] or 0) + (segments["exec_ms"] or 0)
    explique += sum((v or 0) for v in causes.values()) // 1000

    return {
        "total_ms": total,
        "segments": segments,
        "causes_ms": {k: (None if v is None else v // 1000)
                      for k, v in causes.items()},
        "explique_ms": explique,
        "unexplained_ms": total - explique,
        "residu_pct": (100.0 * (total - explique) / total) if total else 0.0,
    }


def imprime_decomposition(instances):
    """`WORKER_COLD_START_BREAKDOWN`, et le meme pour les cinq autres."""
    for i in instances:
        d = decompose(i)
        if d is None:
            print(f"   {i['service']} pid={i['pid']} : bornes insuffisantes"
                  f" (fork_enter ou fin manquants) -- rien a decomposer")
            continue
        etiquette = ("WORKER_COLD_START_BREAKDOWN"
                     if i["service"] == "WebWorker"
                     else "SERVICE_COLD_START_BREAKDOWN")
        print(f"   {etiquette} service={i['service']} pid={i['pid']}"
              f" total_ms={d['total_ms']}")
        for cle, valeur in d["segments"].items():
            print(f"      {cle}={_n(valeur)}")
        for cle, valeur in d["causes_ms"].items():
            print(f"      {cle}={_n(valeur)}")
        print(f"      explique_ms={d['explique_ms']}")
        print(f"      unexplained_ms={d['unexplained_ms']}"
              f" ({d['residu_pct']:.1f} % du total)")
        if d["residu_pct"] > 5.0:
            print(f"      AU-DESSUS DU BUDGET DE 5 % : ce demarrage n'est pas")
            print(f"      explique. Instrumenter avant d'optimiser quoi que ce soit.")
        print()


def imprime_classement(instances):
    """`SERVICE_STARTUP_RANKING`, trie sur la duree MESUREE.

    Le tri est la seule chose qui distingue un classement d'une liste. Trier
    sur l'intuition -- « le worker est le pire » -- est precisement ce qui a
    fait passer a cote des 114 s qui separent l'initialisation de l'hote du
    raccordement de l'interface.
    """
    lignes = []
    for i in instances:
        d = decompose(i)
        if d is None:
            continue
        lignes.append((d["total_ms"], i, d))
    lignes.sort(key=lambda e: -e[0])
    if not lignes:
        print("   (aucune instance decomposable)")
        return
    for total, i, d in lignes:
        print(
            "   SERVICE_STARTUP_RANKING"
            f" service={i['service']}"
            f" pid={i['pid']}"
            f" total_ms={total}"
            f" fork_ms={_n(d['segments']['fork_ms'])}"
            f" exec_ms={_n(d['segments']['exec_ms'])}"
            f" prefault_ms={_n(d['causes_ms']['page_fault_file_ms'])}"
            f" runtime_ms={_n(d['segments']['exec_exit_to_main_ms'])}"
            f" ipc_wait_ms={_n(d['segments']['main_to_ready_ms'])}"
            f" unexplained_ms={d['unexplained_ms']}"
        )


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

    forks, execs, procs, spawns, workers = noyau(source)
    print()
    print("== ce que le noyau a mesure : duplication d'espace ==")
    for ligne in forks[:12] or ["   (aucune)"]:
        print(f"   {ligne}")
    print()
    print("== ce que le noyau a mesure : etapes de l'execve ==")
    for ligne in execs[:12] or ["   (aucune)"]:
        print(f"   {ligne}")
    print()
    print("== les six services, cote a cote ==")
    _INSTANCES = tableau_des_services(execs, forks, spawns, procs, workers)
    imprime_services(_INSTANCES)

    print()
    print("== classement des lancements, trie sur la duree MESUREE ==")
    imprime_classement(_INSTANCES)

    print()
    print("== decomposition de chaque demarrage, residu compris ==")
    imprime_decomposition(_INSTANCES)

    print("== fautes de page par processus, dans le temps ==")
    for ligne in series_par_pid(procs) or ["   (aucune)"]:
        print(f"   {ligne}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
