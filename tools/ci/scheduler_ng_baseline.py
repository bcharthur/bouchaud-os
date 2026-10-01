#!/usr/bin/env python3
"""Agrege les journaux de `scheduler-ng-banc` en une ligne de base.

BOUCHAUD_SCHEDULER_NG_BANC_V1

Entree : des journaux serie de demarrages QEMU (ou physiques) ayant joue
`/bin/scheduler-ng-banc` entre deux `smpstat`. Sortie : un JSON par
configuration (nombre de coeurs) et un tableau Markdown, avec pour chaque
grandeur la MEDIANE des demarrages et leur ETENDUE (min-max) -- une ligne de
base sans dispersion ne permet pas de deriver un seuil.

Ce que le script lit :

  * les lignes `SNG v=1 sec=...` du banc (cle=valeur) ;
  * le DERNIER releve noyau de chaque demarrage : `[SCHED-NG-LAT]`,
    `[SCHED-NG-CENTILES]`, `[SCHED-NG-PREEMPT]`, `[SCHED-NG-REVEIL]`,
    `[SCHED-NG-FILE]`, `[SMP-LOAD]` -- cumules depuis le demarrage, donc
    domines par le banc qui occupe le demarrage.

Fail-closed : un demarrage sans `sec=FIN` est compte `incomplet` et n'entre
dans aucune mediane ; ses lignes partielles restent dans le JSON.

  scheduler_ng_baseline.py --json SORTIE.json --md SORTIE.md LOG...
  scheduler_ng_baseline.py --test
"""
from __future__ import annotations

import argparse
import json
import re
import statistics
import sys
from pathlib import Path

ANSI = re.compile(r"\x1b\[[0-9;]*m")
SNG = re.compile(r"SNG v=1 (.*)$")
CLE_VALEUR = re.compile(r"(\w+)=(\S+)")
NOYAU = {
    "lat": re.compile(r"\[SCHED-NG-LAT\](.*)$"),
    "preempt": re.compile(r"\[SCHED-NG-PREEMPT\](.*)$"),
    "reveil": re.compile(r"\[SCHED-NG-REVEIL\](.*)$"),
}
CENTILES = re.compile(r"\[SCHED-NG-CENTILES\] classe=(\w+)(.*)$")
FILE = re.compile(r"\[SCHED-NG-FILE\] cpu=(\d+)(.*)$")
SMP_LOAD = re.compile(r"\[SMP-LOAD\](.*)$")
STEAL = re.compile(r"\bc(\d+)=\d+ rq=\d+ cur=\S+ steal=(\d+)/(\d+) rej_bal=(\d+) rej_aff=(\d+) rej_inel=(\d+) rej_crs=(\d+) mig=(\d+)")
IDENTITE = re.compile(r"BOUCHAUD_BUILD[^\n]*?commit=([0-9a-f]+)")


def nombre(v: str):
    try:
        return int(v)
    except ValueError:
        try:
            return float(v)
        except ValueError:
            return v


def kv(texte: str) -> dict:
    return {k: nombre(v) for k, v in CLE_VALEUR.findall(texte)}


def cle_mesure(d: dict) -> str | None:
    """Identifiant stable d'une ligne SNG : section + discriminants."""
    sec = d.get("sec")
    if sec == "A":
        return f"A.{d.get('cas')}.c{d.get('calculs')}.{d.get('classe')}"
    if sec == "B":
        return f"B.c{d.get('calculs')}"
    if sec == "C":
        return "C"
    if sec == "D":
        return f"D.{d.get('cas')}"
    if sec == "FIN":
        return "FIN"
    return None


def lit_demarrage(texte: str) -> dict:
    texte = ANSI.sub("", texte)
    mesures: dict[str, dict] = {}
    anomalies = []
    debut = None
    noyau: dict = {"centiles": {}, "file": {}, "vol": {}}
    commit = None
    for ligne in texte.splitlines():
        m = IDENTITE.search(ligne)
        if m and commit is None:
            commit = m.group(1)
        m = SNG.search(ligne)
        if m:
            d = kv(m.group(1))
            if d.get("sec") == "DEBUT":
                debut = d
                continue
            if d.get("sec") == "X":
                anomalies.append(d)
                continue
            cle = cle_mesure(d)
            if cle:
                mesures[cle] = d
            continue
        for nom, motif in NOYAU.items():
            m = motif.search(ligne)
            if m:
                noyau[nom] = kv(m.group(1))
        m = CENTILES.search(ligne)
        if m:
            noyau["centiles"][m.group(1)] = kv(m.group(2))
        m = FILE.search(ligne)
        if m:
            noyau["file"][int(m.group(1))] = kv(m.group(2))
        m = SMP_LOAD.search(ligne)
        if m:
            for c in STEAL.finditer(m.group(1)):
                cpu = int(c.group(1))
                noyau["vol"][cpu] = {
                    "steal": int(c.group(2)), "tentatives": int(c.group(3)),
                    "rej_bal": int(c.group(4)), "rej_aff": int(c.group(5)),
                    "rej_inel": int(c.group(6)), "rej_crs": int(c.group(7)),
                    "mig": int(c.group(8)),
                }
    complet = "FIN" in mesures
    return {
        "coeurs": debut.get("coeurs") if debut else None,
        "commit": commit,
        "complet": complet,
        "mesures": mesures,
        "anomalies": anomalies,
        "noyau": noyau,
    }


def resume_noyau(n: dict) -> dict:
    """Les grandeurs noyau a suivre, aplaties."""
    r = {}
    lat = n.get("lat", {})
    for k in ("count", "max_ns", "interactive_max_ns"):
        if k in lat:
            r[f"lat.{k}"] = lat[k]
    for classe, c in n.get("centiles", {}).items():
        for k in ("p50_ns", "p99_ns", "max_ns"):
            if k in c:
                r[f"attente.{classe}.{k}"] = c[k]
    pre = n.get("preempt", {})
    for k in ("requests", "switches", "max_defer_ns", "attente_service_max_ns"):
        if k in pre:
            r[f"preempt.{k}"] = pre[k]
    files = n.get("file", {}).values()
    if files:
        r["file.doublons"] = sum(f.get("doublons", 0) for f in files)
        r["file.anti_famine"] = sum(f.get("anti_famine", 0) for f in files)
        r["file.volees"] = sum(f.get("volees", 0) for f in files)
    vols = n.get("vol", {}).values()
    if vols:
        r["vol.reussis"] = sum(v["steal"] for v in vols)
        r["vol.tentatives"] = sum(v["tentatives"] for v in vols)
        r["vol.migrations"] = sum(v["mig"] for v in vols)
    return r


# Les grandeurs suivies par configuration (cle de mesure, champ).
SUIVIES = [
    ("A.repos.c0.normale", "p99_us"),
    ("A.charge.c{C}.normale", "p99_us"),
    ("A.charge.c{C}.interactive", "p99_us"),
    ("A.charge.c{2C}.normale", "p99_us"),
    ("A.charge.c{2C}.interactive", "p99_us"),
    ("A.charge.c{2C}.interactive", "max_us"),
    ("B.c2", "jain_milli"),
    ("B.c4", "jain_milli"),
    ("B.c8", "jain_milli"),
    ("B.c16", "jain_milli"),
    ("B.c16", "ratio_min_max_milli"),
    ("B.c16", "trou_max_us"),
    ("C", "reveil_p99_us"),
    ("C", "rtt_p99_us"),
    ("C", "churn_fork_exit"),
    ("D.fork-exit-wait4", "p99_us"),
    ("D.fork-exec-exit", "p99_us"),
    ("D.mort-etrangere-nanosleep", "p99_us"),
    ("D.mort-etrangere-futex", "p99_us"),
    ("D.mort-etrangere-read", "p99_us"),
    ("D.kill-endormi", "p99_us"),
    ("D.futex-pingpong", "echange_moyen_us"),
]

INVARIANTS = [
    ("FIN", "perdus"),
    ("FIN", "ressuscites"),
    ("FIN", "reveils_perdus"),
    ("FIN", "echecs"),
]


def concret(cle: str, coeurs: int) -> str:
    c = min(coeurs, 16)
    c2 = min(2 * coeurs, 16)
    return cle.replace("{2C}", str(c2)).replace("{C}", str(c))


def agrege(demarrages: list[dict]) -> dict:
    par_config: dict[int, list[dict]] = {}
    for d in demarrages:
        if d["coeurs"] is None:
            continue
        par_config.setdefault(int(d["coeurs"]), []).append(d)
    sortie = {}
    for coeurs, ds in sorted(par_config.items()):
        complets = [d for d in ds if d["complet"]]
        lignes = {}
        for cle, champ in SUIVIES + INVARIANTS:
            k = concret(cle, coeurs)
            vals = [d["mesures"][k][champ] for d in complets
                    if k in d["mesures"] and isinstance(d["mesures"][k].get(champ), (int, float))]
            if vals:
                # Le libelle garde le GABARIT : « autant de calculs que de
                # coeurs » est une seule grandeur, quelle que soit la machine.
                libelle = cle.replace("{2C}", "2xC").replace("{C}", "1xC")
                lignes[f"{libelle}:{champ}"] = {
                    "mediane": statistics.median(vals), "min": min(vals), "max": max(vals),
                    "n": len(vals),
                }
        noyaux = [resume_noyau(d["noyau"]) for d in complets]
        for k in sorted({k for n in noyaux for k in n}):
            vals = [n[k] for n in noyaux if isinstance(n.get(k), (int, float))]
            if vals:
                lignes[f"noyau:{k}"] = {
                    "mediane": statistics.median(vals), "min": min(vals), "max": max(vals),
                    "n": len(vals),
                }
        sortie[coeurs] = {
            "demarrages": len(ds),
            "complets": len(complets),
            "commits": sorted({d["commit"] for d in ds if d["commit"]}),
            "anomalies": [a for d in ds for a in d["anomalies"]],
            "lignes": lignes,
        }
    return sortie


def markdown(agr: dict) -> str:
    configs = sorted(agr)
    cles = []
    for c in configs:
        for k in agr[c]["lignes"]:
            if k not in cles:
                cles.append(k)
    out = ["| grandeur | " + " | ".join(f"SMP{c} ({agr[c]['complets']}/{agr[c]['demarrages']})" for c in configs) + " |",
           "|---|" + "---|" * len(configs)]
    for k in cles:
        cellules = []
        for c in configs:
            v = agr[c]["lignes"].get(k)
            if v is None:
                cellules.append("-")
            elif v["min"] == v["max"]:
                cellules.append(f"{v['mediane']:.0f}")
            else:
                cellules.append(f"{v['mediane']:.0f} [{v['min']:.0f}-{v['max']:.0f}]")
        out.append(f"| `{k}` | " + " | ".join(cellules) + " |")
    return "\n".join(out) + "\n"


def auto_test() -> int:
    journal = """\x1b[0mBOUCHAUD_BUILD version=1 commit=9fcc13c59d70
[SCHED-NG-LAT] count=10 avg_ns=5 max_ns=900 interactive_count=2 interactive_max_ns=40 buckets_lt100us=1,lt500us=1,lt2ms=1,lt8ms=1,lt16ms=1,ge16ms=0
SNG v=1 sec=DEBUT coeurs=2 sections=ABCD
SNG v=1 sec=A cas=repos calculs=0 classe=normale n=200 p50_us=10 p95_us=20 p99_us=30 max_us=40
SNG v=1 sec=A cas=charge calculs=2 classe=normale n=200 p50_us=10 p95_us=20 p99_us=300 max_us=400 calcul_tours=1 calcul_jain_milli=999 perdus=0
SNG v=1 sec=B calculs=16 duree_ms=2000 recus=16 tours_min=1 tours_max=2 tours_total=3 jain_milli=900 ratio_min_max_milli=500 trou_max_us=7 trou_median_us=3 perdus=0
SNG v=1 sec=X anomalie=calcul_affame
SNG v=1 sec=D cas=fork-exit-wait4 n=100 ok=100 perdus=0 p50_us=73 p99_us=784 max_us=784
[SCHED-NG-LAT] count=99 avg_ns=5 max_ns=12345 interactive_count=2 interactive_max_ns=40 buckets_lt100us=1,lt500us=1,lt2ms=1,lt8ms=1,lt16ms=1,ge16ms=0
[SCHED-NG-CENTILES] classe=normale count=9 p50_ns=1 p95_ns=2 p99_ns=3 max_ns=4
[SCHED-NG-FILE] cpu=0 interactives=0 normales=0 enfilees=5 doublons=2 defilees=5 volees=1 anti_famine=0
[SMP-LOAD] total=50 tlb=3 c0=50 rq=0 cur=0:0 steal=1/4 rej_bal=2 rej_aff=0 rej_inel=0 rej_crs=0 mig=3 c1=10 rq=0 cur=0:0 steal=2/2 rej_bal=0 rej_aff=0 rej_inel=0 rej_crs=0 mig=1
SNG v=1 sec=FIN coeurs=2 duree_ms=3 echecs=0 perdus=0 ressuscites=0 reveils_perdus=0
"""
    d = lit_demarrage(journal)
    fautes = []
    if not d["complet"] or d["coeurs"] != 2 or d["commit"] != "9fcc13c59d70":
        fautes.append("en-tete mal lu")
    if d["noyau"]["lat"].get("max_ns") != 12345:
        fautes.append("le DERNIER releve noyau doit l'emporter")
    if len(d["anomalies"]) != 1:
        fautes.append("anomalie perdue")
    r = resume_noyau(d["noyau"])
    if r.get("vol.reussis") != 3 or r.get("vol.migrations") != 4 or r.get("file.doublons") != 2:
        fautes.append(f"compteurs de vol/file mal lus : {r}")
    incomplet = lit_demarrage(journal.replace("sec=FIN", "sec=XXX"))
    agr = agrege([d, d, incomplet])
    if agr[2]["complets"] != 2 or agr[2]["demarrages"] != 3:
        fautes.append("un demarrage incomplet est entre dans les medianes")
    if agr[2]["lignes"].get("A.charge.c1xC.normale:p99_us", {}).get("mediane") != 300:
        fautes.append("la charge 1x coeurs n'est pas resolue en c2")
    if "B.c16:trou_max_us" not in agr[2]["lignes"]:
        fautes.append("B.c16 absent")
    md = markdown(agr)
    if "SMP2 (2/3)" not in md:
        fautes.append("en-tete Markdown sans complets/demarrages")
    if fautes:
        print("scheduler_ng_baseline --test : ECHEC")
        print("\n".join("  " + f for f in fautes))
        return 1
    print("SCHEDULER_NG_BASELINE_TEST_OK cas=8")
    return 0


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("journaux", nargs="*", type=Path)
    p.add_argument("--json", type=Path)
    p.add_argument("--md", type=Path)
    p.add_argument("--test", action="store_true")
    a = p.parse_args()
    if a.test:
        return auto_test()
    if not a.journaux:
        p.error("aucun journal")
    demarrages = []
    for j in a.journaux:
        d = lit_demarrage(j.read_text(encoding="utf-8", errors="replace"))
        d["journal"] = str(j)
        demarrages.append(d)
    agr = agrege(demarrages)
    if a.json:
        a.json.write_text(json.dumps({"configs": agr, "demarrages": [
            {k: d[k] for k in ("journal", "coeurs", "commit", "complet", "anomalies")} for d in demarrages
        ]}, indent=1, sort_keys=True), encoding="utf-8")
    md = markdown(agr)
    if a.md:
        a.md.write_text(md, encoding="utf-8")
    else:
        sys.stdout.write(md)
    incomplets = sum(1 for d in demarrages if not d["complet"])
    print(f"SCHEDULER_NG_BASELINE demarrages={len(demarrages)} incomplets={incomplets}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
