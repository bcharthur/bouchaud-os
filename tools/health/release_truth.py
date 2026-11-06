#!/usr/bin/env python3
"""Read-only, fail-closed release evidence for Bouchaud OS.

A successful *report generation* job is NOT a successful release gate.
This script never edits status.json, CI results, or GitHub branch protection.
"""
from __future__ import annotations

import argparse
import datetime as dt
import json
import os
from pathlib import Path
import re
import sys
import urllib.error
import urllib.parse
import urllib.request

FULL_SHA = re.compile(r"^[0-9a-f]{40}$")
REQUIRED_PR_CHECKS = ("CI Fast", "Integration", "Reliability V3", "Native IPC Runtime")
RELEASE_CHECKS = ("ladybird-native-browser", "Nightly Full")
KNOWN_RELEASE_ISSUES = {262, 264}


def github_json(repo: str, path: str, token: str) -> object:
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repo):
        raise ValueError("invalid GitHub repository identifier")
    url = f"https://api.github.com/repos/{repo}/{path}"
    req = urllib.request.Request(
        url,
        headers={
            "Accept": "application/vnd.github+json",
            "User-Agent": "bouchaud-release-evidence/1.0",
            "X-GitHub-Api-Version": "2022-11-28",
            **({"Authorization": f"Bearer {token}"} if token else {}),
        },
    )
    with urllib.request.urlopen(req, timeout=30) as response:
        return json.load(response)


def collect(repo: str, token: str, snapshot_path: Path) -> dict:
    branch = github_json(repo, "branches/main", token)
    sha = branch["commit"]["sha"]
    if not FULL_SHA.fullmatch(sha):
        raise ValueError("main SHA not a 40-character hash")
    runs = github_json(repo, "actions/runs?" + urllib.parse.urlencode(
        {"head_sha": sha, "per_page": 100}
    ), token)
    issues = github_json(repo, "issues?state=open&per_page=100", token)
    if not isinstance(runs.get("workflow_runs"), list) or not isinstance(issues, list):
        raise ValueError("incomplete GitHub API response: fail closed")
    if not snapshot_path.is_file():
        raise FileNotFoundError(f"historical snapshot missing: {snapshot_path}")
    return {
        "main_sha": sha,
        "main_protected": branch.get("protected") is True,
        "workflow_runs": runs["workflow_runs"],
        "issues": [issue for issue in issues if "pull_request" not in issue],
        "snapshot": json.loads(snapshot_path.read_text(encoding="utf-8")),
        "run_list_truncated": bool(runs.get("total_count", 0) > len(runs["workflow_runs"])),
    }


def last_run_for(name: str, sha: str, runs: list) -> dict:
    valid = [
        r for r in runs if r.get("name") == name
        and r.get("head_sha") == sha
        and r.get("event") in ("push", "pull_request", "schedule", "workflow_dispatch")
    ]
    if not valid:
        return {"verdict": "MISSING", "run_id": None}
    # Different workflows may have reruns; inspect the latest *attempt*, not
    # the first historical green run nor a run for a different commit.
    chosen = max(valid, key=lambda r: (r.get("created_at") or "",
                                      int(r.get("run_attempt") or 1),
                                      int(r.get("id") or 0)))
    if chosen.get("status") != "completed":
        status = "PENDING"
    elif chosen.get("conclusion") == "success":
        status = "PASS"
    else:
        status = "FAIL"
    return {
        "verdict": status,
        "run_id": chosen.get("id"),
        "attempt": chosen.get("run_attempt", 1),
        "conclusion": chosen.get("conclusion"),
        "url": chosen.get("html_url"),
        "event": chosen.get("event"),
    }


def evaluate(data: dict, now: str) -> dict:
    sha = data["main_sha"]
    if not FULL_SHA.fullmatch(sha):
        raise ValueError("bad main SHA")
    runs = data["workflow_runs"]
    snap = data["snapshot"]
    ci = {name: last_run_for(name, sha, runs)
          for name in (*REQUIRED_PR_CHECKS, *RELEASE_CHECKS)}
    blockers = []
    for name in REQUIRED_PR_CHECKS:
        if ci[name]["verdict"] != "PASS":
            blockers.append(f"CI:{name}={ci[name]['verdict']}")
    for name in RELEASE_CHECKS:
        if ci[name]["verdict"] != "PASS":
            blockers.append(f"QUALIFICATION:{name}={ci[name]['verdict']}")
    if not data["main_protected"]:
        blockers.append("GOVERNANCE:main_unprotected")
    if snap.get("observed_main_head") != sha:
        blockers.append("SNAPSHOT:stale_sha")
    if snap.get("baseline_status") != "ESTABLISHED":
        blockers.append("BASELINE:not_established")
    if snap.get("physical") not in ("PASS", "QUALIFIED"):
        blockers.append("PHYSICAL:not_qualified")
    if data.get("run_list_truncated"):
        blockers.append("EVIDENCE:workflow_list_truncated")
    issues = [
        {"number": issue.get("number"), "title": issue.get("title"),
         "url": issue.get("html_url")}
        for issue in data["issues"]
        if issue.get("number") in KNOWN_RELEASE_ISSUES
           or (issue.get("title") or "").lower().startswith("bug(")
    ]
    for issue in issues:
        blockers.append(f"ISSUE:#{issue['number']}")
    # An existing strict P13 gate is mandatory. No report can infer it from
    # a green Reliability V3 run or a host-only SMP smoke test.
    p13 = snap.get("p13", {})
    if not isinstance(p13, dict) or p13.get("P13_FINAL_STRICT_OK") is not True:
        blockers.append("P13:strict_unproven_on_this_sha")
    blockers = sorted(set(blockers))
    return {
        "schema": "bouchaud-release-evidence/v1",
        "observed_at_utc": now,
        "repository": "bcharthur/bouchaud-os",
        "exact_main_sha": sha,
        "historical_snapshot_sha": snap.get("observed_main_head"),
        "historical_snapshot_as_of": snap.get("as_of"),
        "snapshot_updates_automatically": bool(snap.get("updates_automatically")),
        "branch_protected": bool(data["main_protected"]),
        "workflow_results": ci,
        "open_critical_issues": issues,
        "release_verdict": "NO_GO" if blockers else "GO_FOR_REVIEW",
        "blockers": blockers,
        "note": "Report generation success is not release approval. No Trigkey flash authorized.",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", default="bcharthur/bouchaud-os")
    parser.add_argument("--status", type=Path, default=Path("docs/current/status.json"))
    parser.add_argument("--output", type=Path, default=Path("target/release-evidence.json"))
    parser.add_argument("--fixture", type=Path, help="offline evidence for regression tests")
    parser.add_argument("--strict", action="store_true",
                        help="fail with code 2 unless all conditions qualify")
    args = parser.parse_args()
    try:
        data = (json.loads(args.fixture.read_text(encoding="utf-8")) if args.fixture
                else collect(args.repo, os.environ.get("GITHUB_TOKEN", ""), args.status))
        now = dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds")
        result = evaluate(data, now)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n",
                               encoding="utf-8")
        print(f"RELEASE_EVIDENCE sha={result['exact_main_sha']} verdict={result['release_verdict']}")
        for reason in result["blockers"]:
            print(f"RELEASE_BLOCKER {reason}")
        if args.strict and result["release_verdict"] != "GO_FOR_REVIEW":
            return 2
        return 0
    except (ValueError, KeyError, TypeError, OSError, urllib.error.URLError) as exc:
        print(f"RELEASE_EVIDENCE_ERROR {type(exc).__name__}: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
