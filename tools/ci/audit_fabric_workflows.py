#!/usr/bin/env python3
"""Compare historical workflow shell steps and their execution parameters."""
import argparse
from collections import Counter
import json
from pathlib import Path
import subprocess

import yaml


BASELINE = "bbce0e7b3ad44e3177174766ef0dcb1e180cd8f8"
DIRECTORY = ".github/workflows"

# Exact historical file and step identities, reviewed with their replacements.
REPLACEMENTS = {
    ("gpu-composition.yml", "Bootimage"):
        "BIOS attesté du run pour le banc GPU et le produit Ladybird",
    ("gpu-composition.yml", "Image de banc -- panne GPU injectee (non livrable)"):
        "build-kernel.yml, variant banc-gpu-composition-panne et manifeste",
    ("native-ipc-runtime.yml", "decision"):
        "déclenchement unique dans bouchaud-ci.yml",
    ("native-ipc-runtime.yml", "Verdict reflete du run push du meme arbre"):
        "verdict IPC et A/B du run courant",
    ("native-ipc-runtime.yml", "Verdict execute"):
        "verdict IPC et A/B du run courant",
    ("os-primitives.yml", "Bootimage"):
        "BIOS attesté du run, construction locale pour les appels autonomes",
    ("p13-primitives-noyau.yml", "decision"):
        "déclenchement P13 dans bouchaud-ci.yml",
    ("p13-primitives-noyau.yml", "Verdict reflete du run push du meme arbre"):
        "résultat P13 du run courant",
    ("p13-primitives-noyau.yml", "Verdict execute"):
        "résultat P13 du run courant",
    ("security-runtime.yml", "Build Bouchaud OS"):
        "BIOS attesté du run",
    ("security-runtime.yml", "Images -- standard, cle-usb-test, temoin"):
        "BIOS attesté, variantes USB construites dans le job",
    ("unix-fs-reliability.yml", "Bootimage"):
        "BIOS attesté du run",
}


def git(*args):
    return subprocess.check_output(["git", *args], text=True)


def shell_steps(document):
    for job in yaml.load(document, Loader=yaml.BaseLoader)["jobs"].values():
        for step in job.get("steps", []):
            if "run" in step:
                yield step


def identity(step):
    return json.dumps(step, sort_keys=True, ensure_ascii=False)


def audit():
    destination = {
        workflow.name: Counter(identity(step) for step in shell_steps(workflow.read_text()))
        for workflow in Path(DIRECTORY).glob("*.yml")
    }
    retained = 0
    unmatched = []
    for name in git("ls-tree", "--name-only", f"{BASELINE}:{DIRECTORY}").splitlines():
        local = destination.get(name, Counter())
        for step in shell_steps(git("show", f"{BASELINE}:{DIRECTORY}/{name}")):
            key = identity(step)
            if local[key]:
                local[key] -= 1
                retained += 1
            else:
                unmatched.append((name, step))

    moved = Counter()
    for remaining in destination.values():
        moved.update(remaining)
    replaced = Counter()
    missing = Counter()
    for name, step in unmatched:
        key = identity(step)
        if moved[key]:
            moved[key] -= 1
            retained += 1
            continue
        label = (name, step.get("name") or step.get("id") or "(sans nom)")
        (replaced if label in REPLACEMENTS else missing)[label] += 1
    print(f"{retained} étapes historiques strictement identiques")
    for label, count in sorted(replaced.items()):
        print(f"{count} × {' / '.join(label)} → {REPLACEMENTS[label]}")
    for label, count in sorted(missing.items()):
        print(f"MANQUANT {count} × {' / '.join(label)}")
    return not missing


if __name__ == "__main__":
    argparse.ArgumentParser(description=__doc__).parse_args()
    raise SystemExit(0 if audit() else 1)
