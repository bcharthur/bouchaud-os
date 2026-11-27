#!/usr/bin/env python3
"""Summarize per-cycle percentiles and wakeup attribution from ABBA archives."""
import collections
import csv
import json
from pathlib import Path
import re
import statistics
import sys
import zipfile


PHASES = r"sous charge \((normal|interactif)\)"
CENTILES = re.compile(PHASES + r" median\s+\d+ us\s+p95\s+(\d+) us\s+p99\s+(\d+) us\s+pire\s+(\d+) us(.*)")
ATTRIBUTION = re.compile(r"attribution " + PHASES + r" : .*?tardif_us=(\d+) attente_us=(\d+) reprise_us=(\d+)")
ATTEMPT = re.compile(r"tentative (\d+)/\d+ : (QUALIFIE_OK|QUALIFIE_ECHEC|NON_QUALIFIANT)")


def cycles(archive):
    with zipfile.ZipFile(archive) as package:
        for name in sorted(package.namelist()):
            match = re.search(r"(?:^|/)(\d+)-(main|candidate)/verdict.json$", name)
            if not match:
                continue
            result = json.loads(package.read(name))
            log = package.read(name.removesuffix("verdict.json") + "smp4.log").decode(errors="replace")
            attempt = 1
            rows = []
            for line in log.splitlines():
                if match_attempt := ATTEMPT.search(line):
                    attempt = int(match_attempt[1]) + 1
                for pattern, kind in ((CENTILES, "centiles"), (ATTRIBUTION, "attribution")):
                    if values := pattern.search(line):
                        phase, a, b, c, *rest = values.groups()
                        population = "propres" if rest and "[propres" in rest[0] else "tous"
                        rows.append((attempt, phase, kind, population, int(a), int(b), int(c)))
            if not rows:
                raise ValueError(f"No phase observations in {archive}:{name}")
            yield match[1], match[2], result["etat"], rows


def main():
    observed = collections.Counter()
    values = collections.defaultdict(list)
    fields = ("archive", "cycle", "arm", "verdict", "attempt", "phase", "kind", "population", "a_us", "b_us", "c_us")
    with Path("diag-observations.csv").open("w", newline="") as output:
        writer = csv.writer(output)
        writer.writerow(fields)
        for archive in map(Path, sys.argv[1:]):
            for cycle, arm, verdict, rows in cycles(archive):
                observed[arm, verdict] += 1
                final_attempt = max(row[0] for row in rows)
                for row in rows:
                    writer.writerow((archive.name, cycle, arm, verdict, *row))
                    attempt, phase, kind, population, *numbers = row
                    if verdict == "OK" and attempt == final_attempt:
                        values[arm, phase, kind, population].append(numbers)
    print("Verdicts:", dict(sorted(observed.items())))
    if observed != {("main", "OK"): 33, ("main", "NON_QUALIFIANT"): 1, ("candidate", "OK"): 34}:
        raise SystemExit("Unexpected archive coverage or cycle verdicts")
    for key, rows in sorted(values.items()):
        fields = ("p95", "p99", "max") if key[2] == "centiles" else ("réveil", "élection", "reprise")
        print(" / ".join(key), f"n={len(rows)}",
              ", ".join(f"{field} min/médiane/max={min(group)}/{statistics.median(group)}/{max(group)} µs"
                        for field, group in zip(fields, zip(*rows))))


if __name__ == "__main__":
    main()
