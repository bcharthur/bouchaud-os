#!/usr/bin/env python3
"""Répète le même scénario WHPX pour les noyaux BASE, GUI, mémoire et candidat."""
import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

from e2e import sha256

ICI = Path(__file__).resolve().parent
SCENARIOS = "E2E-01,E2E-02,E2E-04,E2E-05,E2E-11"
BUILD = re.compile(r"BOUCHAUD_BUILD commit=([a-f0-9]{8,40})\b")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--bras", nargs=2, action="append", required=True,
                    metavar=("SHA_GIT", "BOOTIMAGE"))
    ap.add_argument("--disque", type=Path, required=True)
    ap.add_argument("--sortie", type=Path, required=True)
    ap.add_argument("--qemu", default="qemu-system-x86_64")
    ap.add_argument("--repetitions", type=int, default=3)
    ap.add_argument("--scenarios", default=SCENARIOS)
    ap.add_argument("--politique-mimalloc", default="actuelle")
    o = ap.parse_args()
    if len(o.bras) < 2 or o.repetitions < 2:
        ap.error("au moins deux bras et deux répétitions")
    bras = [(sha, Path(image).resolve()) for sha, image in o.bras]
    if len({sha for sha, _ in bras}) != len(bras):
        ap.error("chaque bras doit avoir son propre SHA")
    if any(not re.fullmatch(r"[a-f0-9]{40}", sha) or not image.is_file() for sha, image in bras):
        ap.error("chaque bras exige un SHA Git complet et une image existante")
    if not o.disque.is_file() or o.sortie.exists():
        ap.error("disque absent ou dossier de sortie déjà présent")
    o.sortie.mkdir(parents=True)
    disque = sha256(o.disque)
    essais = []
    for tour in range(o.repetitions):
        ordre = bras if tour % 2 == 0 else reversed(bras)
        for sha, image in ordre:
            if sha256(o.disque) != disque:
                sys.exit("Image disque initiale modifiée. Campagne interrompue.")
            sortie = o.sortie / f"{tour + 1:02d}-{sha[:12]}"
            commande = [
                sys.executable, str(ICI / "e2e.py"), "--boot", str(image),
                "--disque", str(o.disque), "--accel", "whpx", "--whpx-irqchip", "off",
                "--smp", "1", "--memoire", "12288", "--qemu", o.qemu,
                "--sortie", str(sortie), "--scenarios", o.scenarios,
                "--politique-mimalloc", o.politique_mimalloc,
            ]
            rc = subprocess.run(commande, check=False).returncode
            journal = sortie / "serial.log"
            commits = set(BUILD.findall(journal.read_text(errors="replace"))) if journal.is_file() else set()
            provenance = bool(commits) and all(sha.startswith(commit) for commit in commits)
            essais.append({"tour": tour + 1, "sha_attendu": sha, "sha_journal": sorted(commits),
                           "sha256_bootimage": sha256(image), "provenance": provenance,
                           "code_e2e": rc, "sortie": sortie.name})
            (o.sortie / "campagne.json").write_text(json.dumps({
                "disque_sha256": disque, "scenarios_sha256": sha256(ICI / "scenarios.json"),
                "scenarios": o.scenarios, "essais": essais,
            }, indent=2) + "\n", encoding="utf-8")
    return int(sha256(o.disque) != disque or any(e["code_e2e"] or not e["provenance"] for e in essais))


if __name__ == "__main__":
    sys.exit(main())
