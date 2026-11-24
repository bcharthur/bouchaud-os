#!/usr/bin/env python3
"""Mesures A/B d'un passage du smoke Ladybird (journal serie + sortie smoke).

  mesures_services_ab.py DOSSIER_PASSAGE...   -> une ligne JSON par passage

Temps : horloge de l'INVITE (ms). Un jalon sans horodatage recoit le premier
horodatage invite qui le suit (borne superieure, resolution < 100 ms ici).
"""
import json, re, sys
from pathlib import Path

ANSI = re.compile(r"\x1b\[[0-9;]*m")
HORO = [
    (re.compile(r"BOUCHAUD_BOOT_POINT \S+ t_ms=(\d+)"), 1),
    (re.compile(r"PERF_EXECVE t=(\d+)"), 1),
    (re.compile(r"\[LB:FRAME\] onglet=\d+ seq=\d+ t=(\d+)"), 1),
    (re.compile(r"(?:FAULT_FILE_SNAPSHOT|PERF-PROC\]) t=(\d+)"), 1),
    (re.compile(r"SYSTEM_METRICS .*?monotonic_ns=(\d+)"), 1e-6),
]

def mesure(d: Path):
    serie = next(d.glob("serie-browser-host*.log"))
    lignes = ANSI.sub("", serie.read_bytes().decode("utf-8", "replace")).splitlines()
    smoke = (d / "smoke.log").read_text(errors="replace")
    r = {"passage": d.name}
    en_attente = []
    jalons = {
        "hote_start": "BROWSER_HOST_START",
        "premiere_trame": "BOUCHAUD_UI_FIRST_FRAME onglet=1",
        "document": "[LB:NAV] onglet=1 document_charge",
    }
    ram_doc = None
    attend_ram = False
    for l in lignes:
        t = None
        for motif, echelle in HORO:
            m = motif.search(l)
            if m:
                t = int(int(m.group(1)) * echelle)
                break
        if t is not None and en_attente:
            for nom in en_attente:
                r.setdefault(nom + "_ms", t)
            en_attente = []
        for nom, motif in jalons.items():
            if motif in l and nom + "_ms" not in r and nom not in en_attente:
                en_attente.append(nom)
                if nom == "document":
                    attend_ram = True
        if "PERF_EXECVE" in l and "execve_service0_ms" not in r and t is not None:
            r["execve_service0_ms"] = t
        m = re.search(r"SYSTEM_METRICS .*?ram_used_bytes=(\d+)", l)
        if m:
            used = int(m.group(1))
            r["ram_pic_mio"] = max(r.get("ram_pic_mio", 0), used / 2**20)
            r["ram_fin_mio"] = used / 2**20
            if attend_ram and ram_doc is None:
                ram_doc = used / 2**20
        m = re.search(r"\[BACKING-CACHE\] reads=(\d+) bytes=(\d+).*clean_hit=(\d+) clean_miss=(\d+).*clean_shared=(\d+)", l)
        if m:
            r["disque_mio_fin"] = int(m.group(2)) / 2**20
            r["clean_miss_pages"] = int(m.group(4))
            r["clean_shared_maps"] = int(m.group(5))
            if ram_doc is not None and "disque_mio_doc" not in r:
                r["disque_mio_doc"] = int(m.group(2)) / 2**20
    r["ram_doc_mio"] = ram_doc
    # Somme des RSS fichier des processus Ladybird au dernier releve.
    rss = {}
    for l in lignes:
        m = re.search(r"\[C22:RSS\] t=\d+ pid=(\d+) rss_kio=(\d+) anon_kio=(\d+) fichier_kio=(\d+)", l)
        if m:
            rss[m.group(1)] = (int(m.group(2)), int(m.group(3)), int(m.group(4)))
    m = list(re.finditer(r"releve \d+ t=(\d+) ms : (\d+) lectures, ([\d.]+) Mio", smoke))
    if m:
        r["lectures_attribuees_mio"] = float(m[-1].group(3))
    for k in ("premiere_trame", "document"):
        if k + "_ms" in r and "execve_service0_ms" in r:
            r[k + "_depuis_service0_ms"] = r[k + "_ms"] - r["execve_service0_ms"]
    r["verdict"] = "ok" if "LADYBIRD_FUNCTIONAL_SMOKE ok" in smoke else "echec"
    r["verdicts"] = " ".join(re.findall(r"HOST_[A-Z]+_VERDICT \S+", smoke))
    r["liens"] = (re.findall(r"BO_SMOKE_SERVICES_UNIQUES liens=\d+ octets=\d+", smoke) or ["separes"])[0]
    return r

for a in sys.argv[1:]:
    print(json.dumps(mesure(Path(a)), ensure_ascii=False))
