#!/usr/bin/env python3
"""Start missing independent qualification runs on a verified branch head."""
import argparse
import json
import os
import subprocess
import time

from fabric_merge_gate import PROFILES, QUALIFICATION, latest_run


POLL_INTERVAL_SECONDS = 45
PREREQUISITE_TIMEOUT_SECONDS = 180 * 60


def wait_for_prerequisites(repo, sha, branch, workflows):
    deadline = time.monotonic() + PREREQUISITE_TIMEOUT_SECONDS
    while True:
        pending = []
        for workflow in workflows:
            run = latest_run(repo, sha, branch, workflow)
            if run is None or run["status"] != "completed":
                pending.append(workflow)
            elif run["conclusion"] != "success":
                raise SystemExit(f"Prérequis en échec : {workflow} (run {run['id']})")
        if not pending:
            return
        if time.monotonic() >= deadline:
            raise SystemExit(f"Prérequis encore absents ou en cours : {', '.join(pending)}")
        print(f"Attente des prérequis : {', '.join(pending)}", flush=True)
        time.sleep(POLL_INTERVAL_SECONDS)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("profile", choices=("FULL", "CERTIFICATION"))
    args = parser.parse_args()
    repo, sha, branch = (os.environ[key] for key in (
        "GITHUB_REPOSITORY", "GITHUB_SHA", "FABRIC_PROOF_BRANCH",
    ))
    actual = subprocess.check_output(
        ["gh", "api", f"repos/{repo}/git/ref/heads/{branch}", "--jq", ".object.sha"], text=True,
    ).strip()
    if actual != sha:
        raise SystemExit("La branche de preuve a changé.")
    for workflow in PROFILES[args.profile]["campaigns"]:
        campaign = QUALIFICATION["campaigns"][workflow]
        if latest_run(repo, sha, branch, workflow) is not None:
            print(f"{workflow} : campagne déjà enregistrée sur ce SHA", flush=True)
            continue
        if campaign.get("after"):
            wait_for_prerequisites(repo, sha, branch, campaign["after"])
            if latest_run(repo, sha, branch, workflow) is not None:
                continue
            actual = subprocess.check_output(
                ["gh", "api", f"repos/{repo}/git/ref/heads/{branch}", "--jq", ".object.sha"],
                text=True,
            ).strip()
            if actual != sha:
                raise SystemExit("La branche de preuve a changé pendant l’attente.")
        subprocess.run(
            ["gh", "workflow", "run", workflow, "--repo", repo, "--ref", branch, "--json"],
            input=json.dumps(campaign["dispatch_inputs"]), text=True, check=True,
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
