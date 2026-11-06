#!/usr/bin/env python3
"""Audit live de la gouvernance GitHub (aucun accès écriture au dépôt).

Ce rapport ne confond pas une PR verte et une branche main protégée.
En mode --strict, une protection absente ou un snapshot périmé rend le job rouge.
"""
from __future__ import annotations
import argparse
import json
import os
from pathlib import Path
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
SNAPSHOT = ROOT / "docs/current/status.json"

def conclusions(protected: bool, observed_sha: str, snapshot_sha: str) -> list[str]:
    erreurs: list[str] = []
    if not protected:
        erreurs.append("MAIN_UNPROTECTED: ruleset/branch protection à appliquer par un administrateur")
    if not isinstance(snapshot_sha, str) or len(snapshot_sha) != 40:
        erreurs.append("STATUS_SNAPSHOT_INVALID")
    elif observed_sha != snapshot_sha:
        erreurs.append("STATUS_SNAPSHOT_STALE: snapshot=" + snapshot_sha[:12] +
                       " remote_main=" + observed_sha[:12])
    return erreurs

def github_get(repo: str, endpoint: str) -> dict:
    url = "https://api.github.com/repos/" + repo + "/" + endpoint
    headers = {
        "Accept": "application/vnd.github+json",
        "User-Agent": "bouchaud-os-main-governance-audit",
        "X-GitHub-Api-Version": "2022-11-28",
    }
    token = os.environ.get("GITHUB_TOKEN", "")
    if token:
        headers["Authorization"] = "Bearer " + token
    with urllib.request.urlopen(urllib.request.Request(url, headers=headers), timeout=20) as r:
        data = json.load(r)
    if not isinstance(data, dict):
        raise ValueError("GitHub API: réponse non-objet")
    return data

def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", default="bcharthur/bouchaud-os")
    parser.add_argument("--strict", action="store_true")
    parser.add_argument("--output", type=Path)
    ns = parser.parse_args()
    if ns.repo != "bcharthur/bouchaud-os":
        parser.error("dépôt inattendu (garde anti-configuration accidentelle)")
    try:
        branch = github_get(ns.repo, "branches/main")
        remote_sha = branch["commit"]["sha"]
        protected = branch["protected"]
        snapshot = json.loads(SNAPSHOT.read_text(encoding="utf-8"))
        problems = conclusions(protected, remote_sha, snapshot.get("observed_main_head"))
        if len(remote_sha) != 40 or not isinstance(protected, bool):
            raise ValueError("réponse de protection GitHub invalide")
    except (OSError, ValueError, KeyError, urllib.error.HTTPError, json.JSONDecodeError) as error:
        print(f"GOUVERNANCE_NON_VERIFIEE raison={error}")
        return 2  # Pas de vert sans GitHub accessible.
    report = {
        "repo": ns.repo,
        "remote_main_head": remote_sha,
        "main_protected": protected,
        "status_snapshot_sha": snapshot.get("observed_main_head"),
        "issues": problems,
        "qualification": "AUDIT_ONLY",
        "status": "NEEDS_ACTION" if problems else "CLEAN_AT_INSPECTION",
    }
    if ns.output:
        ns.output.parent.mkdir(parents=True, exist_ok=True)
        ns.output.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n",
                             encoding="utf-8")
    print("GOUVERNANCE_" + report["status"] + " sha=" + remote_sha[:12])
    for problem in problems:
        print("  - " + problem)
    return 2 if problems and ns.strict else 0

if __name__ == "__main__":
    raise SystemExit(main())
