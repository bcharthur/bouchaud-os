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
from fabric_plan import CONFIG, MIGRATED, documentation, plan


class PendingProof(ValueError):
    """The exact campaign has not produced a terminal result."""


def required_profile(base):
    paths = subprocess.check_output(
        ["git", "diff", "--name-only", "--no-renames", "-z", base, "HEAD"],
    ).decode().split("\0")[:-1]
    def textual_documentation(path):
        return documentation(path) and Path(path).suffix in {".md", ".rst", ".txt"}

    if paths and all(map(textual_documentation, paths)):
        return "FAST"
    planned = plan(paths)
    if (not paths or any(path.startswith(CONFIG) for path in paths if not textual_documentation(path))
            or any(documentation(path) and not textual_documentation(path) for path in paths)
            or planned["uncategorized"] or not MIGRATED.intersection(planned["domains"])):
        return "FULL"
    return "FABRIC"


def verify_scoped(repo, head, branch, tested_sha):
    path = f"repos/{repo}/actions"
    runs = api(f"{path}/runs?head_sha={head}&event=pull_request&per_page=100", "workflow_runs")
    matches = [
        run for run in runs if run["head_sha"] == head and run["head_branch"] == branch
        and run["event"] == "pull_request" and run["path"] == ".github/workflows/bouchaud-ci.yml"
    ]
    if not matches:
        raise ValueError("Campagne FABRIC absente sur la tête exacte.")
    run = max(matches, key=lambda item: item["run_number"])
    if run["status"] != "completed":
        raise PendingProof("Dernière campagne FABRIC en cours.")
    if run["conclusion"] != "success":
        raise ValueError("Dernière campagne FABRIC terminée sans succès.")
    report = read_report(path, run, f"fabric-verdict-{run['run_attempt']}", "fabric-verdict.json")
    if report.get("sha") != tested_sha or report.get("profile") != "FABRIC" or report.get("decision") != "PASS_SCOPE":
        raise ValueError("Preuve FABRIC invalide sur l’arbre d’intégration.")
    return {"decision": "GO", "profile": "FABRIC", "sha": head, "run_id": run["id"]}


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
    if run["status"] != "completed":
        raise PendingProof("Dernière campagne FULL en cours.")
    if run["conclusion"] != "success":
        raise ValueError("Dernière campagne FULL terminée sans succès.")
    report = read_report(
        path, run, f"fabric-merge-verdict-{run['run_attempt']}", "fabric-merge-verdict.json",
    )
    expected = {
        "sha": head, "branch": branch, "profile": "FULL", "decision": "FULL_PASS",
        "registry_sha256": hashlib.sha256(REGISTRY.read_bytes()).hexdigest(),
        "run_id": str(run["id"]), "attempt": str(run["run_attempt"]),
    }
    if any(report.get(key) != value for key, value in expected.items()):
        raise ValueError("Identité ou verdict FULL invalide.")
    requirements = PROFILES["FULL"]
    proofs = report["proofs"]
    if set(proofs) != set(requirements["current"]) | set(requirements["campaigns"]) | {"current-archives"}:
        raise ValueError("Couverture FULL incomplète.")
    if any(proof["status"] != "PASS" for proof in proofs.values()):
        raise ValueError("Une preuve FULL obligatoire n’a pas réussi.")
    return {"decision": "GO", "profile": "FULL", **{key: expected[key] for key in ("sha", "run_id", "attempt")}}


def read_report(path, run, expected_name, member_name):
    artifacts = api(f"{path}/runs/{run['id']}/artifacts?per_page=100", "artifacts")
    matches = [item for item in artifacts if item["name"] == expected_name]
    if len(matches) != 1 or not valid_artifact(matches[0], run):
        raise ValueError("Rapport absent, ambigu ou de provenance invalide.")
    artifact = matches[0]
    if artifact["size_in_bytes"] > 1_048_576:
        raise ValueError("Rapport trop volumineux.")
    archive = subprocess.check_output(["gh", "api", f"{path}/artifacts/{artifact['id']}/zip"])
    if artifact["digest"] != f"sha256:{hashlib.sha256(archive).hexdigest()}":
        raise ValueError("Empreinte du rapport invalide.")
    with zipfile.ZipFile(io.BytesIO(archive)) as bundle:
        member = bundle.getinfo(member_name)
        if bundle.namelist() != [member.filename] or member.file_size > 1_048_576:
            raise ValueError("Archive du rapport invalide.")
        return json.loads(bundle.read(member))

def main():
    head = os.environ["PR_HEAD_SHA"]
    profile = "FULL"
    try:
        profile = required_profile(os.environ["PR_BASE_SHA"])
        repo, branch = os.environ["GITHUB_REPOSITORY"], os.environ["PR_HEAD_BRANCH"]
        if profile == "FAST":
            report = {"decision": "GO", "profile": profile, "sha": head, "scope": "documentation"}
        elif profile == "FABRIC":
            report = verify_scoped(repo, head, branch, os.environ["GITHUB_SHA"])
        else:
            report = verify(repo, head, branch)
    except (KeyError, ValueError, subprocess.CalledProcessError, zipfile.BadZipFile) as error:
        decision = "PENDING" if isinstance(error, PendingProof) else "NO-GO"
        report = {"decision": decision, "profile": profile, "sha": head, "reason": str(error)}
    output = json.dumps(report, ensure_ascii=False, indent=2) + "\n"
    print(output, end="")
    Path("fabric-pr-verdict.json").write_text(output)
    return 0 if report["decision"] == "GO" else 1


if __name__ == "__main__":
    raise SystemExit(main())
