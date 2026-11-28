#!/usr/bin/env python3
"""Check independent qualification runs for the exact commit under review."""
import argparse
from collections import Counter
import json
import os
import re
import hashlib
from pathlib import Path
import subprocess


REGISTRY = Path(__file__).with_name("fabric_contracts.json")
QUALIFICATION = json.loads(REGISTRY.read_text())["qualification"]
PROFILES = QUALIFICATION["profiles"]


def api(path, field):
    output = subprocess.check_output(
        ["gh", "api", "--paginate", "--slurp", path], text=True,
    )
    return [item for page in json.loads(output) for item in page[field]]


def latest_run(repo, sha, branch, workflow):
    path = f"repos/{repo}/actions/workflows/{workflow}/runs"
    runs = api(f"{path}?head_sha={sha}&event=workflow_dispatch&per_page=100", "workflow_runs")
    matches = [
        run for run in runs
        if run["head_sha"] == sha and run["head_branch"] == branch
        and run["event"] == "workflow_dispatch"
        and run["path"] == f".github/workflows/{workflow}"
        and (
            workflow != "ladybird-native-browser.yml"
            or (
                run["name"] == "ladybird-native-browser"
                and not (run.get("display_title") or "").startswith("ladybird-artefact ")
            )
        )
    ]
    return max(matches, key=lambda item: item["run_number"], default=None)


def evidence(repo, sha, branch, workflow, required, artifacts=()):
    run = latest_run(repo, sha, branch, workflow)
    if run is None:
        return {"status": "MISSING"}
    proof = {"run": run["id"], "attempt": run["run_attempt"], "status": "FAIL"}
    if run["status"] != "completed" or run["conclusion"] != "success":
        return {**proof, "conclusion": run["conclusion"], "run_status": run["status"]}
    jobs = api(
        f"repos/{repo}/actions/runs/{proof['run']}/jobs?filter=latest&per_page=100",
        "jobs",
    )
    counts = Counter(job["name"] for job in jobs)
    succeeded = {job["name"] for job in jobs if job["status"] == "completed" and job["conclusion"] == "success"}
    missing = [name for name in required if counts[name] != 1 or name not in succeeded]
    archives = artifact_report(repo, run, artifacts) if artifacts and not missing else {}
    return {
        **proof, "status": "FAIL" if missing or archives.get("status") == "FAIL" else "PASS",
        "missing_jobs": missing, "archives": archives,
    }


def artifact_report(repo, run, required, scope_attempt=None):
    listed = api(f"repos/{repo}/actions/runs/{run['id']}/artifacts?per_page=100", "artifacts")
    checked = {}
    missing = []
    for template in required:
        name = template.format(sha=run["head_sha"], attempt=scope_attempt or run["run_attempt"])
        matches = [item for item in listed if item["name"] == name]
        if len(matches) != 1 or not valid_artifact(matches[0], run, since=run.get("created_at")) or not verify_download(repo, matches[0]):
            missing.append(name)
        else:
            checked[name] = {key: matches[0][key] for key in ("id", "digest")}
    return {"status": "FAIL" if missing else "PASS", "missing": missing, "checked": checked}


def valid_artifact(artifact, run, *, since=None):
    source = artifact.get("workflow_run") or {}
    return (
        not artifact.get("expired", True) and artifact.get("size_in_bytes", 0) > 0
        and source.get("id") == run["id"] and source.get("head_sha") == run["head_sha"]
        and re.fullmatch(r"sha256:[0-9a-f]{64}", artifact.get("digest") or "") is not None
        and artifact["created_at"] >= (since or run["run_started_at"])
    )


def verify_download(repo, artifact):
    path = f"repos/{repo}/actions/artifacts/{artifact['id']}/zip"
    with subprocess.Popen(["gh", "api", path], stdout=subprocess.PIPE) as download:
        digest = hashlib.file_digest(download.stdout, "sha256").hexdigest()
        return download.wait() == 0 and artifact["digest"] == f"sha256:{digest}"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("profile", choices=PROFILES)
    args = parser.parse_args()
    sha = os.environ["GITHUB_SHA"]
    branch = os.environ.get("FABRIC_PROOF_BRANCH") or os.environ["GITHUB_REF_NAME"]
    repo = os.environ["GITHUB_REPOSITORY"]
    requirements = PROFILES[args.profile]
    results = {
        variable: {"status": "PASS" if os.environ.get(variable) == "success" else "FAIL"}
        for variable in requirements["current"]
    }
    if "FABRIC_RESULT" in results and os.environ.get("FABRIC_INPUT") != "all":
        results["FABRIC_RESULT"]["status"] = "FAIL"
    if all(proof["status"] == "PASS" for proof in results.values()):
        try:
            run_id = os.environ["GITHUB_RUN_ID"]
            run = json.loads(subprocess.check_output(
                ["gh", "api", f"repos/{repo}/actions/runs/{run_id}"], text=True,
            ))
            if run["head_sha"] != sha or str(run["run_attempt"]) != os.environ["GITHUB_RUN_ATTEMPT"]:
                raise ValueError("Identité du run courant incohérente")
            scope_attempt = os.environ.get("FABRIC_VERDICT_ATTEMPT")
            if "FABRIC_RESULT" in results and not (scope_attempt or "").isdigit():
                raise ValueError("Tentative du verdict Fabric absente")
            results["current-archives"] = artifact_report(
                repo, run, requirements["current_artifacts"], scope_attempt,
            )
        except (KeyError, ValueError, subprocess.CalledProcessError) as error:
            results["current-archives"] = {"status": "ERROR", "type": type(error).__name__}
    for workflow in requirements["campaigns"]:
        campaign = QUALIFICATION["campaigns"][workflow]
        try:
            results[workflow] = evidence(repo, sha, branch, workflow, campaign["jobs"], campaign["artifacts"])
        except (KeyError, ValueError, subprocess.CalledProcessError) as error:
            results[workflow] = {"status": "ERROR", "type": type(error).__name__}
    success = bool(results) and all(proof["status"] == "PASS" for proof in results.values())
    result = {
        "sha": sha, "branch": branch, "profile": args.profile,
        "registry_sha256": hashlib.sha256(REGISTRY.read_bytes()).hexdigest(),
        "run_id": os.environ["GITHUB_RUN_ID"], "attempt": os.environ["GITHUB_RUN_ATTEMPT"],
        "decision": f"{args.profile}_PASS" if success else "NO-GO", "proofs": results,
    }
    output = json.dumps(result, ensure_ascii=False, indent=2) + "\n"
    print(output, end="")
    Path("fabric-merge-verdict.json").write_text(output)
    return 0 if success else 1


if __name__ == "__main__":
    raise SystemExit(main())
