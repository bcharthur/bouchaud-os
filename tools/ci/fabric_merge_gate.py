#!/usr/bin/env python3
"""Check independent qualification runs for the exact commit under review."""
import argparse
from collections import Counter
import json
import os
from pathlib import Path
import subprocess


FULL = {
    "integration.yml": (
        "integration-gate", "kernel / bootimage once", "qemu / mm-ng6 smp4",
        "qemu / os primitives", "qemu / proc-stat et cout du fork",
        "qemu / noyau fige, proprietaire du verrou",
        "qemu / tempete fork+exec (fuite de processus)", "qemu / system health",
        "qemu / boot smoke", "qemu / nvme + gpt", "qemu / python depuis le disque",
        "qemu / fenetre de migration des marques par coeur", "qemu / BRDP + telemetry e2e",
        "qemu / bar pci hors fenetre (uefi, 46 bits)", "qemu / arbre usb",
        "qemu / branchement a chaud", "qemu / stockage usb (bulk-only)", "qemu / liaison volume usb",
        "atelier-reactivite / qemu / reactivite Atelier (KVM, TCG)",
        "dns-udp / qemu / dns udp ring3", "dhcp-recuperation / qemu / dhcp + reprise rx",
    ),
    "reliability-v3.yml": (
        "reliability-gate", "reliability / property fuzz", "reliability / hosted soak 20m",
        *(f"reliability / qemu SMP{cpu}" for cpu in (1, 2, 4, 8)),
    ),
    "ladybird-native-browser.yml": (
        "ladybird / verdict de convergence", "ladybird / provenance de l'artefact",
        "ladybird / C22 memoire 6x6 candidat (KVM strict)", "ladybird / endurance 10 min",
        "ladybird / endurance sous KVM (stabilite + performance)",
        "ladybird / endurance 20 min sous KVM (diagnostic)",
        "ladybird / wpt", "ladybird / cache et SQL apres redemarrage",
        "ladybird / bureau ring 3 (DOOM + Ladybird composes par composited)",
        "ladybird / browser-host smoke", "ladybird / sites reels",
        "ladybird / image Trigkey de livraison (A/B, STATE, bureau, DOOM, Ladybird)",
        "ladybird / robustesse (crash d'un rendu, cadres isoles, cycle des workers)",
    ),
    "nightly.yml": ("nightly-gate", "nightly / Ladybird product", "nightly / dns ring3"),
    "endurance.yml": ("endurance-gate",),
    "ladybird-run-selector-v7.yml": (
        "windows-powershell-51", "live-github-selection", "windows-real-download",
    ),
    "doom-port-windows.yml": ("doom-windows",),
}
PROFILES = {
    "FAST": {},
    "FABRIC": {},
    "FULL": FULL,
    "CERTIFICATION": {
        **FULL,
        "p13-final-candidate.yml": ("P13 / final SHA gate (no missing proof accepted)",),
    },
}


def api(path, field):
    output = subprocess.check_output(
        ["gh", "api", "--paginate", "--slurp", path], text=True,
    )
    return [item for page in json.loads(output) for item in page[field]]


def evidence(repo, sha, branch, workflow, required):
    path = f"repos/{repo}/actions/workflows/{workflow}/runs"
    runs = api(f"{path}?head_sha={sha}&event=workflow_dispatch&per_page=100", "workflow_runs")
    matches = [
        run for run in runs
        if run["head_sha"] == sha and run["head_branch"] == branch
        and run["event"] == "workflow_dispatch"
        and run["path"] == f".github/workflows/{workflow}"
    ]
    if not matches:
        return {"status": "MISSING"}
    run = max(matches, key=lambda item: item["run_number"])
    proof = {"run": run["id"], "attempt": run["run_attempt"], "status": "FAIL"}
    if run["status"] != "completed" or run["conclusion"] != "success":
        return {**proof, "conclusion": run["conclusion"], "run_status": run["status"]}
    jobs = api(
        f"repos/{repo}/actions/runs/{proof['run']}/attempts/{proof['attempt']}/jobs?per_page=100",
        "jobs",
    )
    counts = Counter(job["name"] for job in jobs)
    succeeded = {job["name"] for job in jobs if job["status"] == "completed" and job["conclusion"] == "success"}
    missing = [name for name in required if counts[name] != 1 or name not in succeeded]
    return {**proof, "status": "FAIL" if missing else "PASS", "missing_jobs": missing}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("profile", choices=PROFILES)
    args = parser.parse_args()
    sha = os.environ["GITHUB_SHA"]
    branch = os.environ.get("FABRIC_PROOF_BRANCH") or os.environ["GITHUB_REF_NAME"]
    repo = os.environ["GITHUB_REPOSITORY"]
    results = {}
    if args.profile != "FAST":
        results["fabric-current-run"] = {
            "status": "PASS" if os.environ["FABRIC_RESULT"] == "success"
            and os.environ["FABRIC_INPUT"] == "all" else "FAIL",
        }
    if args.profile in {"FAST", "FULL", "CERTIFICATION"}:
        for name, variable in (("fast", "FAST_RESULT"), ("non-regression", "NON_REGRESSION_RESULT")):
            results[f"{name}-current-run"] = {
                "status": "PASS" if os.environ.get(variable) == "success" else "FAIL",
            }
    if args.profile in {"FULL", "CERTIFICATION"}:
        results["userland-current-run"] = {
            "status": "PASS" if os.environ.get("USERLAND_RESULT") == "success" else "FAIL",
        }
    for workflow, jobs in PROFILES[args.profile].items():
        try:
            results[workflow] = evidence(repo, sha, branch, workflow, jobs)
        except (KeyError, ValueError, subprocess.CalledProcessError) as error:
            results[workflow] = {"status": "ERROR", "type": type(error).__name__}
    success = bool(results) and all(proof["status"] == "PASS" for proof in results.values())
    result = {
        "sha": sha, "branch": branch, "profile": args.profile,
        "decision": f"{args.profile}_PASS" if success else "NO-GO", "proofs": results,
    }
    output = json.dumps(result, ensure_ascii=False, indent=2) + "\n"
    print(output, end="")
    Path("fabric-merge-verdict.json").write_text(output)
    return 0 if success else 1


if __name__ == "__main__":
    raise SystemExit(main())
