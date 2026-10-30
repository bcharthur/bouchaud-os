#!/usr/bin/env python3
"""C22 A/B: reject invalid boots and distinguish true growth from retained RSS.

Strictly positive product gate stays the existing analyse_memoire.py (g2 <= 1024
KiB, stable Compositor). The negative witness can prove the historical defect
as growth or as a REPRODUCIBLE paired RSS retention gap at M2, normalized to M0.
These are two distinct types of evidence; neither is labelled as the other.
"""
from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import asdict, dataclass
from pathlib import Path

# Same 1 MiB uncertainty margin already used by analyse_memoire.py.
# This is a new differential witness criterion, NOT the product leak threshold.
RSS_NOISE_KIB = 1024
PROC_RE = re.compile(r"^MEMOIRE_PROCESSUS pid=(\d+) image=(\S+) m0_kio=(-?\d+) m1_kio=(-?\d+) m2_kio=(-?\d+) g1_kio=(-?\d+) g2_kio=(-?\d+) verdict=(\w+)$", re.M)
PROOF_RE = re.compile(r"PROCESS_SWAPS_PREUVE OK swaps=(\d+)/(\d+) closed=(\d+) nouveaux_webcontent_recoltes=(\d+)")
MARKER_RE = re.compile(r"\[LB:C22\] collecte partitions=(\d+) temoin=([01])")
RUN_RE = re.compile(r"^C22_ESSAI bras=(correctif|temoin) essai=(\d+) rc=(\d+)$", re.M)
COMP_RE = re.compile(r"^MEMOIRE_COMPOSITOR m=([012]) contexts_live=(\d+) backing_stores_live=(\d+) backing_store_kio=(\d+)", re.M)


@dataclass
class Evidence:
    arm: str
    trial: int
    rc: int
    m0: int
    m1: int
    m2: int
    g1: int
    g2: int
    result: str
    collections: int
    proof: str

    @property
    def retained(self) -> int:
        return self.m2 - self.m0


class InvalidProof(ValueError):
    pass


def require(cond: bool, why: str) -> None:
    if not cond:
        raise InvalidProof(why)


def load_run_index(path: Path) -> dict[tuple[str, int], int]:
    require(path.is_file(), f"C22_PREUVE_ABSENTE {path}")
    index: dict[tuple[str, int], int] = {}
    for m in RUN_RE.finditer(path.read_text(encoding="utf-8", errors="replace")):
        key = m.group(1), int(m.group(2))
        require(key not in index, f"C22_DOUBLON {key}")
        index[key] = int(m.group(3))
    return index


def parse_one(root: Path, arm: str, trial: int, rc: int, n: int) -> Evidence:
    summary = root / f"memoire-{arm}-{trial}.sortie"
    serial = root / f"serie-memoire-{arm}-{trial}.log"
    require(summary.is_file(), f"C22_PREUVE_ABSENTE {summary}")
    require(serial.is_file(), f"C22_PREUVE_ABSENTE {serial}")
    output = summary.read_text(encoding="utf-8", errors="replace")
    log = serial.read_text(encoding="utf-8", errors="replace")
    require("MEMOIRE_BOUCLE fini " in output, f"C22_DEMARRAGE_INCOMPLET {arm} {trial}")
    require("KERNEL PANIC" not in log and "PROCESS_FAULT pid=" not in log,
            f"C22_FAUTE_INVITE {arm} {trial}")
    m = PROOF_RE.search(output)
    require(m is not None, f"C22_SWAPS_PREUVE_ABSENTE {arm} {trial}")
    expected = 2 * n
    require(tuple(map(int, m.groups())) == (expected, expected, expected, expected),
            f"C22_SWAPS_INSUFFISANTS {arm} {trial}: {m.group(0)}")
    comps = {int(x.group(1)): tuple(map(int, x.groups()[1:])) for x in COMP_RE.finditer(output)}
    require(set(comps) == {0, 1, 2}, f"C22_COMPOSITOR_REPERES_ABSENTS {arm} {trial}")
    require(all(comps[k] == comps[0] for k in (1, 2)),
            f"C22_COMPOSITOR_NON_STABLE {arm} {trial}: {comps}")
    procs = [x for x in PROC_RE.finditer(output) if x.group(2) == "WebContent" and int(x.group(1)) == 20]
    require(len(procs) == 1, f"C22_WEBCONTENT_PRINCIPAL_INVALIDE {arm} {trial}, n={len(procs)}")
    p = procs[0]
    m0, m1, m2, g1, g2 = map(int, p.groups()[2:7])
    require(g1 == m1 - m0 and g2 == m2 - m1, f"C22_RSS_INCOHERENT {arm} {trial}")
    require("MEMOIRE_INCONCLUSIF" not in output,
            f"C22_RSS_INCONCLUSIF {arm} {trial}")
    native = [(int(x.group(1)), int(x.group(2))) for x in MARKER_RE.finditer(log)]
    require(native, f"C22_TEMOIN_MARQUEURS_ABSENTS {arm} {trial}")
    if arm == "correctif":
        require(all(part > 0 and witness == 0 for part, witness in native if part > 0) and
                not any(witness for _, witness in native) and any(part > 0 for part, _ in native),
                f"C22_CANDIDAT_COLLECTE_NON_PROUVEE {trial}")
        require(rc == 0 and "LADYBIRD_MEMOIRE_OK" in output and
                "MEMOIRE_BORNEE" in output and g2 <= RSS_NOISE_KIB,
                f"C22_CANDIDAT_NON_QUALIFIE {trial}: rc={rc}, g2={g2}")
        proof = "bounded_candidate"
    else:
        require(all(part == 0 and witness == 1 for part, witness in native),
                f"C22_TEMOIN_NON_HISTORIQUE {trial}")
        require(rc in (0, 1), f"C22_TEMOIN_INVALIDE {trial}: rc={rc}")
        if rc == 1:
            require("LADYBIRD_MEMOIRE_ECHEC" in output and
                    "MEMOIRE_CROISSANCE n=" in output and
                    "  ECHEC   memoire :" in output and
                    "MEMOIRE_BORNEE" not in output and
                    len(re.findall(r"^\s+ECHEC\s+", output, flags=re.M)) == 1,
                    f"C22_TEMOIN_ECHEC_NON_MEMOIRE {trial}")
            proof = "historical_growth"
        else:
            require("LADYBIRD_MEMOIRE_OK" in output and "MEMOIRE_BORNEE" in output,
                    f"C22_TEMOIN_RC_INCOHERENT {trial}")
            proof = "bounded_witness"
    return Evidence(arm, trial, rc, m0, m1, m2, g1, g2, p.group(8), len(native), proof)


def analyse(root: Path, repetitions: int, n: int) -> dict:
    require(repetitions in (1, 6), "C22_REPETITIONS_INCORRECTES: 1 pour smoke ou 6 pour strict")
    require(n >= 1, "C22_N_INVALIDE")
    index = load_run_index(root / "c22-validation.txt")
    expected = {(arm, i) for arm in ("correctif", "temoin") for i in range(1, repetitions + 1)}
    require(set(index) == expected, f"C22_CAMPAGNES_INCOMPLETES attendu={sorted(expected)} recu={sorted(index)}")
    pairs = []
    correctifs, temoins = [], []
    for i in range(1, repetitions + 1):
        c = parse_one(root, "correctif", i, index[("correctif", i)], n)
        w = parse_one(root, "temoin", i, index[("temoin", i)], n)
        diff = w.retained - c.retained
        pairs.append({"essai": i, "correctif_retention_kio": c.retained,
                      "temoin_retention_kio": w.retained, "ecart_retention_kio": diff,
                      "temoin_type": w.proof, "temoin_g2_kio": w.g2})
        correctifs.append(c)
        temoins.append(w)
    growth = [w for w in temoins if w.proof == "historical_growth"]
    residual = [p for p in pairs if p["ecart_retention_kio"] > RSS_NOISE_KIB and
                p["temoin_retention_kio"] > RSS_NOISE_KIB]
    # Original historical-growth negative control is still supported.
    # New alternative requires ALL six valid paired witness boots; no proxy
    # for crashes, missing swaps, missing RSS markers, or shorter workload.
    if growth:
        method = "historical_growth"
    elif len(residual) == repetitions:
        method = "paired_retention"
    else:
        raise InvalidProof(f"C22_TEMOIN_NON_DISCRIMINANT paires_valides={len(residual)}/{repetitions} ecarts={[p['ecart_retention_kio'] for p in pairs]}")
    return {"status": "C22_AB_SMOKE_OK" if repetitions == 1 else "C22_AB_CONTRACTUEL_OK",
            "method": method, "repetitions": repetitions, "n_par_phase": n,
            "witness_growth_runs": len(growth), "differential_pairs": len(residual),
            "min_gap_kio": min(p["ecart_retention_kio"] for p in pairs),
            "pairs": pairs,
            "notice": "paired_retention proves a stable memory footprint gap, not an unbounded memory leak"}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--results", type=Path, default=Path("."))
    ap.add_argument("--repetitions", type=int, choices=[1, 6], default=6)
    ap.add_argument("--n-par-phase", type=int, default=10)
    ap.add_argument("--json", type=Path, default=None)
    args = ap.parse_args()
    try:
        result = analyse(args.results, args.repetitions, args.n_par_phase)
    except (InvalidProof, OSError, ValueError) as exc:
        print("C22_AB_NOT_ESTABLISHED", str(exc), file=sys.stderr)
        if args.json:
            args.json.write_text(json.dumps({"status": "C22_AB_NOT_ESTABLISHED", "reason": str(exc)}, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        return 1
    for p in result["pairs"]:
        print(f"C22_AB_PAIR essai={p['essai']} correctif_retention={p['correctif_retention_kio']}KiB "
              f"temoin_retention={p['temoin_retention_kio']}KiB ecart={p['ecart_retention_kio']}KiB "
              f"temoin_mode={p['temoin_type']}")
    print(f"{result['status']} methode={result['method']} "
          f"paires={result['differential_pairs']}/{args.repetitions} "
          f"temoins_croissance={result['witness_growth_runs']} "
          f"ecart_min_kio={result['min_gap_kio']}")
    if args.json:
        args.json.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
