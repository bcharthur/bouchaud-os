#!/usr/bin/env python3
"""Treat a missing required job as a qualification failure."""

import json
import os
from pathlib import Path

from fabric_plan import MIGRATED


STATUS = {
    "success": "PASS",
    "failure": "FAIL",
    "cancelled": "CANCELLED",
    "skipped": "NOT_RUN",
}


def verdict(needs):
    plan = needs.get("plan", {})
    plan_result = plan.get("result")
    outputs = plan.get("outputs", {})
    report = {"plan": STATUS.get(plan_result, "NOT_RUN")}
    flags = {name: outputs.get(name) for name in MIGRATED}
    coherent = all(flag in ("true", "false") for flag in flags.values())
    if coherent:
        coherent = outputs.get("any") == str(any(flags[name] == "true" for name in ("gpu", "security", "storage", "p13", "memory"))).lower()
    if plan_result == "success" and not coherent:
        report["plan"] = "FAIL"

    for name in ("kernel", "gpu-witness", *sorted(MIGRATED)):
        result = needs.get(name, {}).get("result")
        key = {"kernel": "any", "gpu-witness": "gpu"}.get(name, name)
        flag = outputs.get(key)
        if report["plan"] != "PASS":
            report[name] = "BLOCKED"
        elif flag == "true":
            report[name] = STATUS.get(result, "NOT_RUN")
        elif result == "skipped":
            report[name] = "SKIPPED_BY_PLAN"
        else:
            report[name] = "FAIL"
    return report


def main():
    results = verdict(json.loads(os.environ["NEEDS_JSON"]))
    go = all(result in ("PASS", "SKIPPED_BY_PLAN") for result in results.values())
    report = {
        "sha": os.environ["GITHUB_SHA"],
        "profile": "FABRIC",
        "run": f"{os.environ['GITHUB_SERVER_URL']}/{os.environ['GITHUB_REPOSITORY']}/actions/runs/{os.environ['GITHUB_RUN_ID']}",
        "decision": "PASS_SCOPE" if go else "NO-GO",
        "global_qualification": "NOT_RUN",
        "results": results,
    }
    output = json.dumps(report, indent=2)
    print(output)
    Path("fabric-verdict.json").write_text(output + "\n")
    return 0 if go else 1


if __name__ == "__main__":
    raise SystemExit(main())
