"""Structural coverage shared by the P13 and Ladybird policy checks."""
import json

import yaml


def p13_errors(workflow, compatibility, primitives, required=()):
    config = yaml.load(workflow, Loader=yaml.BaseLoader)
    events = config.get("on", {})
    paths = set(json.loads(compatibility).get("p13", []))
    errors = []
    pr, push = events.get("pull_request"), events.get("push")
    if (
        "pull_request" not in events or pr not in (None, "")
        or push != {"branches": ["main"], "tags": ["ci-fabric/**"]}
    ):
        errors.append("P13: déclencheurs automatiques PR et main non couverts")
    jobs = config["jobs"]
    expected = {
        "needs": ["plan", "kernel"],
        "if": "needs.plan.outputs.p13 == 'true'",
        "uses": "./.github/workflows/os-primitives.yml",
        "with": {
            "p13_strict": "false",
            "cpu_only": "true",
            "bootimage_artifact": "${{ needs.kernel.outputs.artifact }}",
        },
    }
    if jobs.get("p13") != expected or "p13" not in jobs["verdict"]["needs"]:
        errors.append("P13: suite ou dépendance de verdict absente")
    memory = {
        "needs": ["plan", "kernel"],
        "if": "needs.plan.outputs.memory == 'true'",
        "uses": "./.github/workflows/fabric-memory.yml",
        "with": {"bootimage_artifact": "${{ needs.kernel.outputs.artifact }}"},
    }
    if jobs.get("memory") != memory or "memory" not in jobs["verdict"]["needs"]:
        errors.append("Memory Fabric: suite ou dépendance de verdict absente")
    missing = set(required) - paths
    if missing:
        errors.append(f"P13: chemins historiques non couverts {sorted(missing)}")
    if not primitives.strip():
        errors.append("P13: workflow des primitives absent")
    return errors
