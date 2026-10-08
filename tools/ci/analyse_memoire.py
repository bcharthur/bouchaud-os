#!/usr/bin/env python3
"""Memoire apres deux phases identiques, avec preuves fraiches au repos.

Les evenements context_create/destroy ne sont PAS des instantanes au repere.
Selection dans l'ordre du journal, strictement AVANT chaque repere : deux
evenements dans la meme seconde ne sont pas simultanes. Trois echantillons
periodiques recents et stables sont requis, avec bilan creations-destructions.
Baseline Compositor stricte ; RSS : plateau a un Mio de bruit pres.
La croissance de la phase precedente ne constitue pas un budget de fuite.
Sans les preuves requises, MEMOIRE_INCONCLUSIF (2), jamais MEMOIRE_BORNEE.
"""
import re
import sys
from pathlib import Path

ANSI = re.compile(r"\x1b\[[0-9;]*m")
HEURE = re.compile(r"^\[(\d\d):(\d\d):(\d\d)\]")
REPERE = re.compile(r"HOST_MEMOIRE_REPERE m=([012]) n=(\d+)")
PROC = re.compile(r"\[PERF-PROC\] t=\d+ pid=(\d+) image=(\S+) rss_kio=(\d+) vss_kio=\d+ taches=")
MEM = re.compile(r"\[LB:MEM\] ev=sample ctx=0 contexts_live=(\d+) backing_stores_live=(\d+) backing_store_octets=(\d+) skia_ressources_octets=\d+ skia_ressources_limite=\d+ skia_polices_octets=\d+ created_total=(\d+) destroyed_total=(\d+) END")
REQUIRED = {"Compositor", "ImageDecoder", "RequestServer", "WebContent"}


def secondes(line):
    m = HEURE.match(line)
    return int(m[1]) * 3600 + int(m[2]) * 60 + int(m[3]) if m else None


def age(now, then):
    return (now - then) % 86400


def analyse(text):
    reperes, rss, mem = {}, {}, []
    lines = ANSI.sub("", text).splitlines()
    for i, line in enumerate(lines):
        t = secondes(line)
        if t is None:
            continue
        if m := REPERE.search(line):
            reperes.setdefault(int(m[1]), (i, t, int(m[2])))
        if m := PROC.search(line):
            name = Path(m[2]).name
            rss.setdefault((int(m[1]), name), []).append((i, t, int(m[3])))
        if m := MEM.search(line):
            mem.append((i, t, tuple(map(int, m.groups()))))
    if set(reperes) != {0, 1, 2} or not reperes[0][0] < reperes[1][0] < reperes[2][0]:
        return 2, [f"MEMOIRE_INCONCLUSIF reperes={sorted(reperes)}"]
    if len({v[2] for v in reperes.values()}) != 1 or reperes[0][2] < 1:
        return 2, ["MEMOIRE_INCONCLUSIF phases non identiques"]

    report, missing, growth = [], [], 0
    snapshots = []
    for k in (0, 1, 2):
        index, time, _ = reperes[k]
        recent = [v for i, t, v in mem if i < index and age(time, t) <= 5][-3:]
        if len(recent) != 3 or len(set(recent)) != 1:
            missing.append(f"Compositor M{k}: trois releves frais et stables absents")
            continue
        value = recent[-1]
        live, stores, size, created, destroyed = value
        if created < destroyed or created - destroyed != live:
            missing.append(f"Compositor M{k}: bilan contextes incoherent")
        snapshots.append(value)
        report.append(f"MEMOIRE_COMPOSITOR m={k} contexts_live={live} backing_stores_live={stores} backing_store_kio={size//1024} created={created} destroyed={destroyed}")
    if len(snapshots) == 3:
        baseline = snapshots[0]
        for k, v in enumerate(snapshots[1:], 1):
            # Aucun +1 arbitraire : tous les objets des onglets doivent partir.
            if any(v[j] > baseline[j] for j in range(3)):
                growth += 1
                report.append(f"MEMOIRE_COMPOSITOR_OBJETS_RESTANTS m={k} baseline={baseline[:3]} mesure={v[:3]}")
            if v[3] < snapshots[k-1][3] or v[4] < snapshots[k-1][4]:
                missing.append("Compositor: compteurs remis a zero")

    covered = set()
    for (pid, name), points in sorted(rss.items()):
        # Meme processus avant M0 et apres M2 ; jamais un renderer temporaire.
        if points[0][0] >= reperes[0][0] or points[-1][0] <= reperes[2][0]:
            continue
        values = []
        for k in (0, 1, 2):
            index, time, _ = reperes[k]
            recent = [r for i, t, r in points if i < index and age(time, t) <= 10]
            if not recent:
                break
            values.append(recent[-1])
        if len(values) != 3:
            missing.append(f"RSS {name} pid={pid}: releve frais manquant")
            continue
        covered.add(name)
        m0, m1, m2 = values
        g1, g2 = m1 - m0, m2 - m1
        verdict = "croissance" if g2 > 1024 else "borne"
        growth += verdict == "croissance"
        report.append(f"MEMOIRE_PROCESSUS pid={pid} image={name} m0_kio={m0} m1_kio={m1} m2_kio={m2} g1_kio={g1} g2_kio={g2} verdict={verdict}")
    if REQUIRED - covered:
        missing.append(f"services RSS absents: {sorted(REQUIRED-covered)}")
    if missing:
        return 2, report + ["MEMOIRE_INCONCLUSIF " + "; ".join(missing)]
    if growth:
        return 1, report + [f"MEMOIRE_CROISSANCE n={growth}"]
    return 0, report + ["MEMOIRE_BORNEE"]


if __name__ == "__main__":
    code, report = analyse(Path(sys.argv[1]).read_text(errors="replace"))
    print("\n".join(report))
    sys.exit(code)
