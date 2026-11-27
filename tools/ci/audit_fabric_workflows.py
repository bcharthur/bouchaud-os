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
JOB_MOVES = {
    ("integration.yml", "atelier-reactivite"): ("fabric-atelier.yml", "atelier-reactivite"),
    ("integration.yml", "dns-udp"): ("fabric-network.yml", "dns-udp"),
    ("integration.yml", "dhcp-recuperation"): ("fabric-network.yml", "dhcp-recuperation"),
    ("native-ipc-runtime.yml", "ab-rollback"): ("system-update.yml", "ab-rollback"),
}

# Exact historical file and step identities, reviewed with their replacements.
REPLACEMENTS = {
    ("ladybird-native-browser.yml", "QEMU + outils"):
        "préparation perf-surface à partir des sources Ubuntu signées",
    ("native-ipc-runtime.yml", "QEMU + OVMF"):
        "préparation A/B à partir des sources Ubuntu signées",
    ("nightly.yml", "Dependances"):
        "préparation QEMU core à partir des sources Ubuntu signées",
    ("ci.yml", "Classer les changements"):
        "classification exhaustive explicite pour le profil FAST",
    ("ci.yml", "Verdict unique pour branch protection"):
        "verdict historique et succès de chaque job exigé en mode exhaustif",
    ("gpu-composition.yml", "Bootimage"):
        "BIOS attesté du run pour le banc GPU et le produit Ladybird",
    ("gpu-composition.yml", "Image de banc -- panne GPU injectee (non livrable)"):
        "build-kernel.yml, variant banc-gpu-composition-panne et manifeste",
    ("ladybird-native-browser.yml", "Bootimage"):
        "BIOS attesté du run, distinct de l'artefact natif Ladybird",
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
    ("userland.yml", "Publier release du commit"):
        "publication autonome conservée, campagne Fabric sans publication",
    ("unix-fs-reliability.yml", "Bootimage"):
        "BIOS attesté du run",
}


def git(*args):
    return subprocess.check_output(["git", *args], text=True)


def load(document):
    return yaml.load(document, Loader=yaml.BaseLoader)


def shell_steps(document):
    for job in load(document)["jobs"].values():
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


def graph():
    """Expose changes outside shell steps for review against the pinned baseline."""
    jobs = ("needs", "if", "strategy", "runs-on", "timeout-minutes", "uses", "permissions")
    destination = {p.name: load(p.read_text()) for p in Path(DIRECTORY).glob("*.yml")}
    intact = True
    for filename in git("ls-tree", "--name-only", f"{BASELINE}:{DIRECTORY}").splitlines():
        historical = load(git("show", f"{BASELINE}:{DIRECTORY}/{filename}"))
        current = destination.get(filename, {})
        if historical["on"] != current.get("on"):
            print(f"{filename}: déclencheurs modifiés")
        for name, before in historical["jobs"].items():
            after = current.get("jobs", {}).get(name)
            target = JOB_MOVES.get((filename, name))
            if target:
                target_file, target_job = target
                moved = destination[target_file]["jobs"][target_job]
                fixed = ("strategy", "runs-on", "timeout-minutes", "continue-on-error")
                exact = all(before.get(field) == moved.get(field) for field in fixed)
                moved_steps = moved.get("steps")
                if target_file == "system-update.yml":
                    moved_steps = [
                        {**step, "run": step["run"].replace("apt-installe-ubuntu.sh", "apt-installe.sh")}
                        if step.get("name") == "QEMU + OVMF" else step
                        for step in moved_steps
                    ]
                exact &= before.get("steps") == moved_steps
                intact &= exact
                print(f"{filename}/{name} → {target_file}/{target_job}: "
                      f"{'matrice, runner et étapes couverts' if exact else 'écart à vérifier'}")
            if after is None:
                print(f"{filename}/{name}: job déplacé ou remplacé")
                continue
            differences = [field for field in jobs if before.get(field) != after.get(field)]
            before_artifacts = artifact_steps(before)
            after_artifacts = artifact_steps(after)
            if before_artifacts != after_artifacts:
                differences.append("artefacts")
            if differences:
                print(f"{filename}/{name}: {', '.join(differences)}")
    return intact


def artifact_steps(job):
    return [
        step for step in job.get("steps", [])
        if step.get("uses", "").startswith(("actions/upload-artifact@", "actions/download-artifact@"))
    ]


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--graph", action="store_true")
    options = parser.parse_args()
    passed = audit()
    if options.graph:
        passed = graph() and passed
    raise SystemExit(0 if passed else 1)
