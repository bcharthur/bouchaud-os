#!/usr/bin/env python3
"""Start missing independent qualification runs on a verified branch head."""
import argparse
import json
import os
import subprocess

from fabric_merge_gate import PROFILES, QUALIFICATION, latest_run


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
        if any(
            (latest_run(repo, sha, branch, preceding) or {}).get("conclusion") != "success"
            for preceding in campaign.get("after", [])
        ):
            print(f"{workflow} : en attente de ses prérequis", flush=True)
            continue
        subprocess.run(
            ["gh", "workflow", "run", workflow, "--repo", repo, "--ref", branch, "--json"],
            input=json.dumps(campaign["dispatch_inputs"]), text=True, check=True,
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
