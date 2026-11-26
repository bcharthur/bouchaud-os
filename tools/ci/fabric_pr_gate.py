#!/usr/bin/env python3
"""Verify a completed FULL qualification for the exact source tree of a PR."""
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import zipfile

from fabric_merge_gate import PROFILES, REGISTRY, api, valid_artifact


def verify(repo, head, branch):
    def tree(commit):
        return subprocess.check_output(
            ["git", "rev-parse", "--verify", f"{commit}^{{tree}}"], text=True,
        ).strip()

    if tree(head) != tree("HEAD"):
        raise ValueError("L’arbre de fusion diffère de la tête qualifiée.")
    ref = f"ci-fabric/FULL/{head}/{branch}"
    path = f"repos/{repo}/actions"
    runs = api(f"{path}/runs?head_sha={head}&event=push&per_page=100", "workflow_runs")
    matches = [run for run in runs if run["head_sha"] == head and run["head_branch"] == ref
               and run["event"] == "push" and run["path"] == ".github/workflows/bouchaud-ci.yml"]
    if not matches:
        raise ValueError("Preuve FULL par tag absente sur la tête exacte.")
    run = max(matches, key=lambda item: item["run_number"])
    if run["status"] != "completed" or run["conclusion"] != "success":
        raise ValueError("Dernière campagne FULL en attente ou sans succès.")
    expected_name = f"fabric-merge-verdict-{run['run_attempt']}"
    artifacts = api(f"{path}/runs/{run['id']}/artifacts?per_page=100", "artifacts")
    matches = [item for item in artifacts if item["name"] == expected_name]
    if len(matches) != 1 or not valid_artifact(matches[0], run):
        raise ValueError("Rapport FULL absent, ambigu ou de provenance invalide.")
    artifact = matches[0]
    if artifact["size_in_bytes"] > 1_048_576:
        raise ValueError("Rapport FULL trop volumineux.")
    archive = subprocess.check_output(["gh", "api", f"{path}/artifacts/{artifact['id']}/zip"])
    if artifact["digest"] != f"sha256:{hashlib.sha256(archive).hexdigest()}":
        raise ValueError("Empreinte du rapport FULL invalide.")
    with zipfile.ZipFile(io.BytesIO(archive)) as bundle:
        member = bundle.getinfo("fabric-merge-verdict.json")
        if bundle.namelist() != [member.filename] or member.file_size > 1_048_576:
            raise ValueError("Archive du rapport FULL invalide.")
        report = json.loads(bundle.read(member))
    expected = {
        "sha": head, "branch": branch, "profile": "FULL", "decision": "FULL_PASS",
        "registry_sha256": hashlib.sha256(REGISTRY.read_bytes()).hexdigest(),
        "run_id": str(run["id"]), "attempt": str(run["run_attempt"]),
    }
    if any(report.get(key) != value for key, value in expected.items()):
        raise ValueError("Identité ou verdict FULL invalide.")
    requirements = PROFILES["FULL"]
    proofs = report["proofs"]
    if set(proofs) != set(requirements["current"]) | set(requirements["campaigns"]):
        raise ValueError("Couverture FULL incomplète.")
    if any(proof["status"] != "PASS" for proof in proofs.values()):
        raise ValueError("Une preuve FULL obligatoire n’a pas réussi.")
    return {"decision": "GO", "profile": "FULL", **{key: expected[key] for key in ("sha", "run_id", "attempt")}}


def main():
    head = os.environ["PR_HEAD_SHA"]
    try:
        report = verify(os.environ["GITHUB_REPOSITORY"], head, os.environ["PR_HEAD_BRANCH"])
    except (KeyError, ValueError, subprocess.CalledProcessError, zipfile.BadZipFile) as error:
        report = {"decision": "NO-GO", "profile": "FULL", "sha": head, "reason": str(error)}
    output = json.dumps(report, ensure_ascii=False, indent=2) + "\n"
    print(output, end="")
    Path("fabric-pr-verdict.json").write_text(output)
    return 0 if report["decision"] == "GO" else 1


if __name__ == "__main__":
    raise SystemExit(main())
