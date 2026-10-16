#!/usr/bin/env python3
"""Rapport de diagnostic P13 sur des traces originales (répertoire ou ZIP CI).

Ce rapport N'EST PAS un verdict de qualification : il ne remplace jamais
qualifie_p13.py, ni une preuve sous KVM sur le SHA exact.
"""
from __future__ import annotations

import argparse
import json
import re
import sys
import zipfile
from collections import Counter
from pathlib import Path

ANSI = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")
HOST_PREFIX = re.compile(r"^(?:\[[^\]]*\]){2}\[FPS:[^\]]*\] ")
VERDICT = re.compile(r"^VERDICT_SONDE : (QUALIFIE_OK|QUALIFIE_ECHEC|NON_QUALIFIANT)\b(.*)")
COMPTE = re.compile(r"\bpreuve_hote=(kvm-steal|absente)\b.*?\bprouves=(\d+)\b.*?\binexpliques=(\d+)\b")
CORR = re.compile(r"\bPROC_STAT_MONOTONIE_CORRECTION\b")
FIN = re.compile(r"^ORDO_ATTRIBUTION_FIN$")
FAULT = re.compile(r"\b(KERNEL PANIC|panicked at|ASSERTION FAILED)\b")
ATTRIB_PATH = re.compile(r"(?:^|/)ordonnanceur-attribution-kvm-smp(1|2|4|8)/serie\.log$")
AB_PATH = re.compile(r"(?:^|/)ab-reveil-precis-kvm-smp(1|4)/serie\.log$")


def normalise(lines: str) -> list[str]:
    return [HOST_PREFIX.sub("", ANSI.sub("", line.rstrip("\r"))) for line in lines.splitlines()]


def collecte(archive: Path) -> dict[str, str]:
    traces: dict[str, str] = {}
    if archive.is_file() and zipfile.is_zipfile(archive):
        with zipfile.ZipFile(archive) as z:
            for item in z.infolist():
                if item.is_dir() or item.file_size > 20_000_000:
                    continue
                if ATTRIB_PATH.search(item.filename) or AB_PATH.search(item.filename):
                    traces[item.filename] = z.read(item).decode("utf-8", "replace")
    elif archive.is_dir():
        for p in archive.rglob("serie.log"):
            relative = p.relative_to(archive).as_posix()
            if ATTRIB_PATH.search(relative) or AB_PATH.search(relative):
                traces[relative] = p.read_text(encoding="utf-8", errors="replace")
    elif archive.is_file():
        traces[archive.name] = archive.read_text(encoding="utf-8", errors="replace")
    else:
        raise FileNotFoundError(f"source absente : {archive}")
    if not traces:
        raise ValueError("aucune trace P13 reconnue dans la source")
    return traces


def diagnostique(path: str, raw: str) -> dict:
    lines = normalise(raw)
    verdicts = [m.group(1) for line in lines if (m := VERDICT.match(line))]
    motifs = Counter()
    for line in lines:
        if VERDICT.match(line):
            if "charge sans effet mesurable" in line:
                motifs["charge_non_discriminante"] += 1
            if "phase non jugeable" in line and ("hote" in line or "hôte" in line):
                motifs["contamination_hote"] += 1
            if "sans explication" in line or "sans-preuve" in line:
                motifs["retards_non_attribues"] += 1
        if "PREUVE la charge degrade la latence" in line:
            if "= faux" in line:
                motifs["porte_charge_echouee"] += 1
    compteurs = []
    for line in lines:
        if "comptes" in line and (m := COMPTE.search(line)):
            compteurs.append({"preuve_hote": m.group(1), "retards_hote": int(m.group(2)), "retards_inexpliques": int(m.group(3))})
    panic = any(FAULT.search(x) for x in lines)
    fin = any(FIN.match(x) for x in lines)
    m = ATTRIB_PATH.search(path)
    kind, smp = ("attribution", int(m.group(1))) if m else ("ab", int(AB_PATH.search(path).group(1))) if AB_PATH.search(path) else ("inconnu", None)
    # Un verdict d'analyse conserve le statut brut ; jamais un SUCCESS déduit du workflow parent.
    qualified = (kind == "attribution" and fin and len(verdicts) == 2
                 and all(x == "QUALIFIE_OK" for x in verdicts) and not panic
                 and len(compteurs) >= 6 and all(x["preuve_hote"] == "kvm-steal" for x in compteurs))
    return {
        "trace": path, "type": kind, "smp": smp,
        "verdicts": dict(Counter(verdicts)), "passes": len(verdicts),
        "fin_executee": fin if kind == "attribution" else None,
        "panique_ou_assertion": panic,
        "causes": dict(motifs),
        "phases_hote": len(compteurs),
        "retards_proves_hote": sum(x["retards_hote"] for x in compteurs),
        "retards_inexpliques": sum(x["retards_inexpliques"] for x in compteurs),
        "corrections_proc_stat_visibles": sum(bool(CORR.search(x)) for x in lines),
        "qualification_attribution_indicative": "QUALIFIE_OK" if qualified else "NON_ETABLIE",
    }


def rapport(source: Path, sha: str | None = None) -> dict:
    traces = collecte(source)
    records = [diagnostique(k, v) for k, v in sorted(traces.items())]
    return {"source": str(source), "sha_declare": sha, "attention": "Diagnostic seulement. Le SHA est fourni par l'appelant, non prouvé par les journaux.",
            "resultats": records, "baseline_p13": "NOT_ESTABLISHED"}


def markdown(data: dict) -> str:
    out = ["# Diagnostic P13 — preuves collectées", "", f"Source : `{data['source']}`", f"SHA déclaré (non vérifié) : `{data['sha_declare'] or 'inconnu'}`", "",
           "**Ce rapport n'accorde jamais une qualification globale P13.**", "", "| Trace | Verdicts | Cause dominante | Hôte prouvé / inexpliqué |", "|---|---|---|---|"]
    for row in data["resultats"]:
        verdicts = ", ".join(f"{k}={v}" for k, v in row["verdicts"].items()) or "aucun"
        cause = ", ".join(f"{k}={v}" for k, v in row["causes"].items()) or "non catégorisée"
        out.append(f"| `{row['type']} SMP{row['smp'] or '?'}` | {verdicts} | {cause} | {row['retards_proves_hote']} / {row['retards_inexpliques']} |")
    out += ["", "Prochaines preuves : qualification stricte avec `qualifie_p13.py` sur KVM dédié, endurance SMP8 distincte et campagnes indépendantes de même SHA.", ""]
    return "\n".join(out)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("source", type=Path, help="ZIP os-primitives-debug ou dossier d'artefact")
    ap.add_argument("--sha", help="SHA attendu (déclaratif, pas authentifié par le ZIP)")
    ap.add_argument("--json", type=Path, dest="json_out")
    ap.add_argument("--markdown", type=Path, dest="md_out")
    ns = ap.parse_args()
    try:
        data = rapport(ns.source, ns.sha)
    except (OSError, ValueError, zipfile.BadZipFile) as ex:
        print(f"P13_DIAGNOSTIC_ERREUR {ex}", file=sys.stderr)
        return 2
    if ns.json_out:
        ns.json_out.parent.mkdir(parents=True, exist_ok=True)
        ns.json_out.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    if ns.md_out:
        ns.md_out.parent.mkdir(parents=True, exist_ok=True)
        ns.md_out.write_text(markdown(data), encoding="utf-8")
    print(markdown(data))
    print("P13_RAPPORT_TERMINE baseline=NOT_ESTABLISHED")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
