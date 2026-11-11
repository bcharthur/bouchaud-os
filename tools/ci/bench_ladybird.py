#!/usr/bin/env python3
"""BOUCHAUD_LADYBIRD_BENCH_V1 : performance utilisateur de Ladybird, A/B
composition GPU on/off, tiree des journaux serie du smoke produit REEL.

  bench_ladybird.py SERIE_GPU_ON SERIE_GPU_OFF

Deux demarrages a froid, meme noyau, meme artefact Ladybird, meme page,
meme peripherique (`virtio-gpu-gl-pci`) ; seul `gpu-composition on|off`
change. Chaque ligne dit d'ou vient la valeur ; une mesure absente est
ecrite `n/m`, jamais inventee. Ce qui n'est PAS instrumente (latence
entree -> presentation, defilement) est dit tel quel.
"""
import re
import statistics
import sys
from pathlib import Path

ANSI = re.compile(r"\x1b\[[0-9;]*m")


def texte(p):
    try:
        return ANSI.sub("", Path(p).read_bytes().decode("utf-8", "replace"))
    except OSError:
        return ""


def premier(t, motif):
    m = re.search(motif, t)
    return int(m[1]) if m else None


def tous(t, motif):
    return [int(v) for v in re.findall(motif, t)]


def med(v):
    v = [x for x in v if x is not None]
    return statistics.median(v) if v else None


def releves(t, balise):
    out = []
    for l in t.splitlines():
        if balise in l:
            out.append({k: int(v) for k, v in re.findall(r"(\w+)=(\d+)", l.split(balise, 1)[1])})
    # Le premier releve couvre le demarrage : on l'ecarte S'IL Y EN A D'AUTRES.
    # Un smoke court n'en produit qu'un par bras ; l'ecarter vidait le banc
    # (tout `n/m`) et faisait conclure a tort que le bras GPU n'avait rien
    # pose. Le nombre de releves retenus est imprime (`releves_gpu_composition`).
    return out[1:] if len(out) > 1 else out


def mesures(t):
    m = {}
    m["boot_bureau_ms"] = premier(t, r"BOUCHAUD_BOOT_POINT bureau-premiere-trame t_ms=(\d+)")
    m["lancement_premiere_image_ms"] = premier(t, r"PERF_FIRST_PAINT perf_id=\d+ t_ns=\d+ since_click_ms=(\d+)")
    m["premiere_trame_client_ms"] = premier(t, r"premiere trame du client pid=\d+ \(\d+x\d+\) apres (\d+) ms")
    m["webcontent_demarrage_ms"] = med(tous(t, r"SERVICE_STARTUP_RANKING service=WebContent pid=\d+ total_ms=(\d+)"))
    m["webworker_demarrage_ms"] = med(tous(t, r"SERVICE_STARTUP_RANKING service=WebWorker pid=\d+ total_ms=(\d+)"))
    m["execve_services_us"] = med(tous(t, r"PERF_EXECVE t=\d+ image=/usr/libexec/ladybird/\w+ pid=\d+ duree_us=(\d+)"))
    m["fork_copie_us"] = med(tous(t, r"PERF_FORK t=\d+ pere=\d+ enfant=\d+ image=\S+ duree_us=\d+ copie_us=(\d+)"))
    m["js_batterie_workers_ms"] = premier(t, r"HOST_WORKER_BATTERIE OK \d+/\d+ ms=(\d+)")
    m["js_microtaches_ms"] = premier(t, r"HOST_JS nom=queueMicrotask-ordre etat=OK ms=(\d+)")
    m["reseau_fetch_worker_ms"] = premier(t, r"HOST_WORKER_TEST nom=fetch-depuis-worker etat=OK ms=(\d+)")
    m["reseau_worker_http_premier_ms"] = premier(t, r"HOST_WORKER_HTTP_PERF_FIRST OK ms=(\d+)")
    rss = {}
    for pid, kio in re.findall(r"\[C22:RSS\] t=\d+ pid=(\d+) rss_kio=(\d+)", t):
        rss[pid] = max(rss.get(pid, 0), int(kio))
    m["rss_max_par_processus_kio"] = max(rss.values()) if rss else None
    m["rss_somme_des_max_kio"] = sum(rss.values()) if rss else None
    cache = re.findall(r"PAGE_CACHE_GLOBAL scope=global hits=(\d+) miss=(\d+)", t)
    m["cache_pages_miss"] = int(cache[-1][1]) if cache else None
    rg = releves(t, "[GPU-COMPOSITION]")
    m["releves_gpu_composition"] = len(rg)
    m["dessin_cpu_us_par_trame"] = med([r.get("us_dessin_cpu_par_trame") for r in rg])
    m["px_client_copies_cpu_30s"] = med([r.get("px_client_copies_cpu") for r in rg])
    m["px_poses_gpu_30s"] = med([r.get("px_poses_gpu") for r in rg])
    m["trames_bureau_30s"] = med([r.get("trames_bureau") for r in rg])
    m["presentation_us_par_trame"] = med([r.get("us_presentation_par_trame") for r in rg])
    tr = [l for l in t.splitlines() if "[GPU-TRAMES]" in l]
    p = re.search(r"presentation_us_p50=(\d+) p95=(\d+) p99=(\d+) max=(\d+)", tr[-1]) if tr else None
    m["presentation_p50_p95_p99_max_us"] = "/".join(p.groups()) if p else None
    d = re.search(r"dessin_cpu_us_p50=(\d+) p95=(\d+) p99=(\d+) max=(\d+)", tr[-1]) if tr else None
    m["dessin_p50_p95_p99_max_us"] = "/".join(d.groups()) if d else None
    m["smoke_ok"] = int("LADYBIRD_BROWSER_HOST_OK" in t or "BROWSER_HOST_OK" in t)
    return m


LIBELLES = [
    ("boot_bureau_ms", "Démarrage noyau → première trame du bureau", "ms"),
    ("lancement_premiere_image_ms", "Lancement du navigateur → première image", "ms"),
    ("premiere_trame_client_ms", "Processus navigateur → première trame cliente", "ms"),
    ("webcontent_demarrage_ms", "Démarrage à froid de WebContent", "ms"),
    ("webworker_demarrage_ms", "Démarrage à froid d'un WebWorker (médiane)", "ms"),
    ("execve_services_us", "execve d'un service Ladybird (médiane)", "µs"),
    ("fork_copie_us", "fork : copie des pages (médiane)", "µs"),
    ("js_batterie_workers_ms", "JS : batterie de 10 tests Worker", "ms"),
    ("js_microtaches_ms", "JS : ordre microtâches/promesses", "ms"),
    ("reseau_fetch_worker_ms", "Réseau : fetch depuis un Worker", "ms"),
    ("reseau_worker_http_premier_ms", "Réseau : premier Worker HTTP", "ms"),
    ("rss_max_par_processus_kio", "Mémoire : RSS max d'un processus", "Kio"),
    ("rss_somme_des_max_kio", "Mémoire : somme des RSS max", "Kio"),
    ("cache_pages_miss", "Défauts du cache de pages (fin)", ""),
    ("dessin_cpu_us_par_trame", "Dessin CPU du bureau par trame", "µs"),
    ("px_client_copies_cpu_30s", "Pixels du navigateur recopiés par le CPU / 30 s", "px"),
    ("px_poses_gpu_30s", "Pixels posés par le GPU / 30 s", "px"),
    ("trames_bureau_30s", "Trames du bureau / 30 s", ""),
    ("releves_gpu_composition", "Relevés [GPU-COMPOSITION] retenus (1 = inclut le démarrage)", ""),
    ("presentation_us_par_trame", "Présentation par trame (attente, le fil dort)", "µs"),
    ("presentation_p50_p95_p99_max_us", "Présentation p50/p95/p99/max", "µs"),
    ("dessin_p50_p95_p99_max_us", "Dessin CPU p50/p95/p99/max", "µs"),
]


def f(v, u):
    if v is None:
        return "n/m"
    if isinstance(v, str):
        return f"≤ {v} {u}".strip()
    return (f"{v:,.0f}".replace(",", " ") + (f" {u}" if u else "")).strip()


def main():
    on, off = texte(sys.argv[1]), texte(sys.argv[2])
    if not on or not off:
        print("LADYBIRD_BENCH_ECHEC journal absent")
        return 1
    a, b = mesures(on), mesures(off)
    print("| Mesure (Ladybird réel, smoke produit) | GPU on | GPU off |")
    print("|---|---:|---:|")
    for cle, nom, u in LIBELLES:
        print(f"| {nom} | {f(a[cle], u)} | {f(b[cle], u)} |")
    print("| Latence entrée → présentation | non instrumentée | non instrumentée |")
    print("| Défilement, trames perdues | non instrumenté | non instrumenté |")
    for cle, _, _ in LIBELLES:
        print(f"LADYBIRD_BENCH {cle} on={a[cle]} off={b[cle]}")
    # Le banc n'a de sens que si le bras GPU a compose par le GPU et que le
    # bras CPU ne l'a pas fait : sinon ce n'est pas un A/B.
    if not (a["px_poses_gpu_30s"] or 0) > 0:
        print("LADYBIRD_BENCH_ECHEC le bras GPU n'a pas compose par le GPU")
        return 1
    if (b["px_poses_gpu_30s"] or 0) > 0:
        print("LADYBIRD_BENCH_ECHEC le bras CPU a compose par le GPU")
        return 1
    print("LADYBIRD_BENCH_OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
