#!/usr/bin/env python3
"""BOUCHAUD_PERF_SESSION_V1 -- ce qu'une session Ladybird a coute, lu dans son journal serie.

    analyse_session.py JOURNAL [--json SORTIE]          une session
    analyse_session.py --compare RUN_A RUN_B [...]      A/B de parcours E2E

JOURNAL : le journal serie d'une session -- parcours E2E (`serial.log`) ou
session interactive (`run-reference-stage2-ladybird.ps1`, `QEMU_SERIAL=`).
RUN_x : un dossier de sortie de `tools/e2e/ladybird/e2e.py`.

## Ce que l'analyse rend

  horloge     la source du TSC, le controle contre les ticks et le verdict.
              UNE HORLOGE NON COHERENTE INVALIDE LES DUREES DE L'INVITE : la
              comparaison est alors refusee (code 3), jamais publiee.
  coeurs      les coeurs que l'ordonnanceur a mis EN LIGNE, pas ceux demandes
              a QEMU (`-smp`).
  processus   par image (WebContent, RequestServer...) : CPU utilisateur et
              noyau, RSS, fautes et leur cout -- dont les fautes A ZERO.
  appels      les appels systeme les plus emis, cumules (madvise,
              clock_gettime...).
  rendu       composition GPU disponible ou non, trames.
  pauses      PERSIST-SLOW (> 1 s), timeouts du parseur HTML.

Rien n'est invente : une mesure absente du journal reste `null`.
"""
from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

ANSI = re.compile(r"\x1b\[[0-9;]*m")
# Les lignes portent un prefixe d'horodatage et de charge : on cherche les
# marqueurs n'importe ou dans la ligne.
R_TSC = re.compile(r"BOUCHAUD_TSC_CONTROLE hz=(\d+) source=(\S+) hz_selon_ticks=(\d+) rapport_pour_mille=(\d+)")
R_QUALITE = re.compile(r"BOUCHAUD_M12_CLOCK_QUALITY status=(\S+) ratio_per_mille=(\d+) source=(\S+)")
R_TSC_TOT = re.compile(r"BOUCHAUD_TSC_EARLY_CALIBRATION_OK hz=(\d+) source=(\S+)")
R_HPET = re.compile(r"BOUCHAUD_HPET_MONOTONIC_READY")
R_EN_LIGNE = re.compile(r"SMP_NG2_SCHEDULER online=(\d+)")
R_PERF_PROC = re.compile(
    r"\[PERF-PROC\] t=(\d+) pid=(\d+) image=(\S+) rss_kio=(\d+) vss_kio=(\d+) taches=(\d+) "
    r"fautes=(\d+) total_us=(\d+) pire_us=(\d+) zero=(\d+)/(\d+)us fichier=(\d+)/(\d+)us "
    r"partage=(\d+)/(\d+)us copie=(\d+)/(\d+)us attente=(\d+)/(\d+)us echec=(\d+) user_ms=(\d+) sys_ms=(\d+)")
R_PROC_SAMPLE = re.compile(r"\[PROC-SAMPLE\] v=1 t_ns=(\d+) pid=(\d+) name=(\S+) cpu_pct=(\d+)")
R_SYSCALL_TOP = re.compile(r"\[SYSCALL-TOP\] window_ns=(\d+) appels=(\d+) distincts=\d+ cumul=(\d+)(.*)")
R_APPEL = re.compile(r" ([a-z_0-9?]+)=(\d+)/eagain=(\d+)")
R_GPU = re.compile(r"\[GPU-COMPOSITION\] disponible=(\d+) periode_ms=\d+ trames_bureau=(\d+) trames_clients=(\d+) trames_gpu=(\d+)")
R_PERSIST = re.compile(r"\[PERSIST-SLOW\] n=\d+ total_ns=(\d+)")
R_PARSEUR = re.compile(r"HTMLParserEndState")
R_PREMIERE = re.compile(r"BOUCHAUD_UI_FIRST_FRAME onglet=(\d+)")
R_NAV = re.compile(r"\[LB:NAV\] onglet=(\d+) document_charge url=(\S+)")
R_MOLETTE = re.compile(r"\[LB:FRAME\] onglet=\d+ seq=\d+ t=\d+ apres_molette=1")


def lit(chemin: Path) -> str:
    return ANSI.sub("", chemin.read_text(errors="replace")).replace("\r", "")


def horloge(texte: str) -> dict:
    """Verdict de coherence de l'horloge monotone de l'invite.

    `coherente` : une frequence TSC est retenue ET le controle contre les
    ticks PIT est dans +-10 %. Hors de cette bande, les durees mesurees dans
    l'invite ne sont pas comparables entre deux sessions.
    """
    tot = R_TSC_TOT.search(texte)
    ctrl = R_TSC.search(texte)
    qual = R_QUALITE.search(texte)
    rapport = int(ctrl.group(4)) if ctrl else (int(qual.group(2)) if qual else None)
    hz = int(ctrl.group(1)) if ctrl else (int(tot.group(1)) if tot else None)
    source = ctrl.group(2) if ctrl else (tot.group(2) if tot else (qual.group(3) if qual else None))
    if hz in (None, 0):
        verdict, raison = "inconnue", "aucune frequence TSC retenue : l'horloge monotone repose sur le HPET ou les ticks"
    elif rapport is None:
        verdict, raison = "inconnue", "controle TSC/ticks absent du journal"
    elif 900 <= rapport <= 1100:
        verdict, raison = "coherente", ""
    else:
        verdict, raison = "incoherente", f"TSC et ticks divergent : rapport {rapport} pour mille"
    return {"hz": hz, "source": source, "rapport_pour_mille": rapport, "hpet_pret": bool(R_HPET.search(texte)),
            "qualite_noyau": qual.group(1) if qual else None, "verdict": verdict, "raison": raison}


def analyse(texte: str) -> dict:
    en_ligne = [int(x) for x in R_EN_LIGNE.findall(texte)]
    # Par image, le DERNIER releve (cumuls) de chaque pid ; puis la somme des
    # pid d'une meme image (un WebContent par onglet).
    derniers: dict[str, list] = {}
    for m in R_PERF_PROC.finditer(texte):
        derniers[m.group(2)] = m.groups()
    images: dict[str, dict] = {}
    for g in derniers.values():
        im = images.setdefault(g[2], {"pids": 0, "rss_kio_max": 0, "fautes": 0, "fautes_us": 0, "zero": 0,
                                      "zero_us": 0, "fichier": 0, "copie": 0, "attente_us": 0,
                                      "user_ms": 0, "sys_ms": 0})
        im["pids"] += 1
        im["rss_kio_max"] = max(im["rss_kio_max"], int(g[3]))
        im["fautes"] += int(g[6])
        im["fautes_us"] += int(g[7])
        im["zero"] += int(g[9])
        im["zero_us"] += int(g[10])
        im["fichier"] += int(g[11])
        im["copie"] += int(g[15])
        im["attente_us"] += int(g[18])
        im["user_ms"] += int(g[20])
        im["sys_ms"] += int(g[21])
    cpu: dict[str, list[int]] = {}
    for m in R_PROC_SAMPLE.finditer(texte):
        cpu.setdefault(m.group(3), []).append(int(m.group(4)))
    appels: dict[str, int] = {}
    eagain: dict[str, int] = {}
    fenetres_ns = 0
    for m in R_SYSCALL_TOP.finditer(texte):
        fenetres_ns += int(m.group(1))
        for nom, n, e in R_APPEL.findall(m.group(4)):
            appels[nom] = appels.get(nom, 0) + int(n)
            eagain[nom] = eagain.get(nom, 0) + int(e)
    gpu = R_GPU.findall(texte)
    persist = [int(x) for x in R_PERSIST.findall(texte)]
    return {
        "horloge": horloge(texte),
        "coeurs_en_ligne": en_ligne[-1] if en_ligne else None,
        "processus": images,
        "cpu_pct_moyen": {k: round(sum(v) / len(v)) for k, v in cpu.items()},
        "cpu_pct_pic": {k: max(v) for k, v in cpu.items()},
        "appels": dict(sorted(appels.items(), key=lambda kv: -kv[1])[:12]),
        "appels_eagain": {k: eagain[k] for k in list(dict(sorted(appels.items(), key=lambda kv: -kv[1])[:12]))},
        "appels_fenetre_s": round(fenetres_ns / 1e9, 1),
        "gpu_disponible": (gpu[-1][0] == "1") if gpu else None,
        "trames_gpu": int(gpu[-1][3]) if gpu else None,
        "persist_lent": {"n": sum(1 for x in persist if x >= 1_000_000_000),
                         "pire_ms": max(persist) // 1_000_000 if persist else None},
        "parseur_html_timeouts": len(R_PARSEUR.findall(texte)),
        "premiere_trame": bool(R_PREMIERE.search(texte)),
        "documents_charges": len(R_NAV.findall(texte)),
        "trames_apres_molette": len(R_MOLETTE.findall(texte)),
    }


def resume(a: dict) -> str:
    h = a["horloge"]
    l = [f"horloge          : {h['verdict']} (source={h['source']} hz={h['hz']} rapport={h['rapport_pour_mille']}/1000)"
         + (f" -- {h['raison']}" if h["raison"] else ""),
         f"coeurs en ligne  : {a['coeurs_en_ligne']}"]
    for nom, p in sorted(a["processus"].items(), key=lambda kv: -(kv[1]["user_ms"] + kv[1]["sys_ms"])):
        if p["user_ms"] + p["sys_ms"] == 0 and p["fautes"] == 0:
            continue
        l.append(f"{nom:<17}: user={p['user_ms']} ms sys={p['sys_ms']} ms rss_max={p['rss_kio_max'] // 1024} Mio "
                 f"fautes={p['fautes']} ({p['fautes_us'] // 1000} ms) dont zero={p['zero']} ({p['zero_us'] // 1000} ms)"
                 f" cpu_moy={a['cpu_pct_moyen'].get(nom, '-')}%")
    if a["appels"]:
        l.append("appels systeme   : " + " ".join(f"{k}={v}" for k, v in a["appels"].items())
                 + f" (sur {a['appels_fenetre_s']} s releves)")
    l.append(f"composition GPU  : disponible={a['gpu_disponible']} trames_gpu={a['trames_gpu']}")
    l.append(f"pauses           : persist>1s={a['persist_lent']['n']} (pire {a['persist_lent']['pire_ms']} ms) "
             f"timeouts_parseur_html={a['parseur_html_timeouts']}")
    return "\n".join(l)


# --- A/B de parcours E2E -----------------------------------------------------

METRIQUES_AB = [
    ("E2E-02", "clic_jusqu_a_la_premiere_trame_ms"),
    ("E2E-02", "clic_jusqu_a_l_accueil_charge_ms"),
    ("E2E-04", "entree_jusqu_aux_resultats_ms"),
    ("E2E-04", "lien_jusqu_a_l_article_ms"),
    ("E2E-11", "saisie_jusqu_au_document_lourd_ms"),
    ("E2E-11", "saisie_jusqu_a_l_image_utile_ms"),
    ("E2E-11", "clic_carte_jusqu_a_la_video_ms"),
    ("E2E-11", "retour_jusqu_a_l_image_utile_ms"),
    ("E2E-11", "molette_1_jusqu_a_la_trame_ms"),
    ("E2E-11", "molette_2_jusqu_a_la_trame_ms"),
    ("E2E-11", "molette_3_jusqu_a_la_trame_ms"),
]


def lit_run(dossier: Path) -> dict:
    manifeste = json.loads((dossier / "manifest.json").read_text())
    resultats = json.loads((dossier / "results.json").read_text())
    liste = resultats if isinstance(resultats, list) else resultats.get("resultats", [])
    par_id = {r["id"]: r for r in liste}
    session = analyse(lit(dossier / "serial.log"))
    return {"dossier": str(dossier), "manifeste": manifeste, "par_id": par_id, "session": session}


def etiquette(run: dict) -> str:
    env = run["manifeste"].get("environnement", {})
    return f"{env.get('accel_demande', '?')}/smp{env.get('smp', '?')}/{env.get('politique_mimalloc', '?')}"


def compare(dossiers: list[Path]) -> int:
    runs = [lit_run(d) for d in dossiers]
    refus = [(etiquette(r), r["session"]["horloge"]) for r in runs if r["session"]["horloge"]["verdict"] != "coherente"]
    if refus:
        for nom, h in refus:
            print(f"PERF_AB_REFUS run={nom} horloge={h['verdict']} raison={h['raison']}")
        print("PERF_AB_REFUS comparaison non publiee : une horloge non coherente rend les durees incomparables")
        return 3
    for r in runs:
        print(f"== {etiquette(r)}")
        print(resume(r["session"]))
    print()
    noms = [etiquette(r) for r in runs]
    largeur = max(len(n) for n in noms) + 2
    print("metrique".ljust(40) + "".join(n.rjust(largeur) for n in noms))
    for sid, m in METRIQUES_AB:
        vals = []
        for r in runs:
            v = r["par_id"].get(sid, {}).get("metriques", {}).get(m)
            vals.append("-" if v is None else str(v))
        print(m.ljust(40) + "".join(v.rjust(largeur) for v in vals))
    lignes = [("coeurs_en_ligne", lambda s: s["coeurs_en_ligne"]),
              ("webcontent_fautes_zero", lambda s: s["processus"].get("WebContent", {}).get("zero")),
              ("webcontent_zero_ms", lambda s: (s["processus"].get("WebContent", {}).get("zero_us") or 0) // 1000),
              ("webcontent_sys_ms", lambda s: s["processus"].get("WebContent", {}).get("sys_ms")),
              ("webcontent_user_ms", lambda s: s["processus"].get("WebContent", {}).get("user_ms")),
              ("webcontent_rss_max_mio", lambda s: (s["processus"].get("WebContent", {}).get("rss_kio_max") or 0) // 1024),
              ("madvise", lambda s: s["appels"].get("madvise")),
              ("clock_gettime", lambda s: s["appels"].get("clock_gettime"))]
    for nom, f in lignes:
        print(nom.ljust(40) + "".join(str(f(r["session"]) if f(r["session"]) is not None else "-").rjust(largeur)
                                      for r in runs))
    return 0


def main(argv: list[str]) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("journal", nargs="?", type=Path)
    p.add_argument("--json", type=Path)
    p.add_argument("--compare", nargs="+", type=Path)
    o = p.parse_args(argv[1:])
    if o.compare:
        return compare(o.compare)
    if not o.journal:
        p.error("JOURNAL ou --compare")
    a = analyse(lit(o.journal))
    print(resume(a))
    if o.json:
        o.json.write_text(json.dumps(a, indent=1, ensure_ascii=False))
    return 0 if a["horloge"]["verdict"] == "coherente" else 3


if __name__ == "__main__":
    sys.exit(main(sys.argv))
