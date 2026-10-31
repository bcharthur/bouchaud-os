#!/usr/bin/env python3
"""Fail-closed eligibility of a Ladybird artifact for the *single* C22 smoke.

A red memory diagnostic is not a qualified Ladybird run. It may nevertheless
supply a successfully built, independently verified native binary to the
one-shot smoke. The 6x6 qualification gate is deliberately NOT changed.
"""
import json
import re
import sys
from pathlib import Path

BUILD = "ladybird / build once"
REUSED = "ladybird / artefact reutilise (entrees identiques)"
AVAILABLE = "ladybird / artefact disponible"
MEMORY = "ladybird / memoire apres onglets fermes (diagnostic)"
VERDICT = "ladybird / verdict integration developpement"
ALLOWED_RED = {MEMORY, VERDICT}
ALLOWED_SKIPPED = {"primitives", BUILD, REUSED}
ARTIFACT = "bouchaud-ladybird-native-browser"


def verify(run, job_data, artifact_data, expected_run_id, repository):
    def require(condition, message):
        if not condition:
            raise ValueError("C22_NATIVE_PREFLIGHT_REFUSE: " + message)

    require(isinstance(run, dict) and run.get("id") == int(expected_run_id),
            "identifiant du run incoherent")
    require(run.get("name") == "ladybird-native-browser"
            and run.get("status") == "completed"
            and run.get("conclusion") in ("success", "failure"),
            "producteur Ladybird absent, incomplet ou annule")
    require(run.get("repository", {}).get("full_name") == repository
            and run.get("head_repository", {}).get("full_name") == repository,
            "depot producteur different")
    require(run.get("head_branch") == "chatgpt/active"
            and run.get("event") in ("push", "workflow_dispatch"),
            "branche ou evenement producteur inattendu")
    require(re.fullmatch(r"[0-9a-f]{40}", run.get("head_sha", "")) is not None,
            "SHA producteur invalide")

    jobs = job_data.get("jobs", [])
    require(isinstance(jobs, list) and 0 < len(jobs) <= 100\n            and job_data.get("total_count", len(jobs)) == len(jobs),
            "liste de jobs absente ou tronquee")
    by_name = {job.get("name"): job for job in jobs}
    require(len(by_name) == len(jobs), "noms de jobs dupliques")
    require(all(job.get("status") == "completed" for job in jobs),
            "job encore actif ou incomplet")
    require(by_name.get(AVAILABLE, {}).get("conclusion") == "success",
            "liaison de l'artefact non reussie")
    require(any(by_name.get(name, {}).get("conclusion") == "success"
                for name in (BUILD, REUSED)),
            "ni build natif ni reutilisation prouvee")
    require(VERDICT in by_name, "verdict d'integration absent")

    failures = {name for name, job in by_name.items()
                if job.get("conclusion") not in ("success", "skipped")}
    require(failures <= ALLOWED_RED,
            "echec inattendu: " + ", ".join(sorted(failures - ALLOWED_RED)))
    skipped = {name for name, job in by_name.items()
               if job.get("conclusion") == "skipped"}
    require(skipped <= ALLOWED_SKIPPED,
            "job inattendu saute: " + ", ".join(sorted(skipped - ALLOWED_SKIPPED)))
    if run["conclusion"] == "failure":
        require(failures == ALLOWED_RED,
                "run rouge sans preuve d'un echec limite au diagnostic memoire")
    else:
        require(not failures and by_name[VERDICT]["conclusion"] == "success",
                "run vert mais jobs incoherents")

    artifacts = artifact_data.get("artifacts", [])
    require(isinstance(artifacts, list), "liste d'artefacts invalide")
    native = [a for a in artifacts if a.get("name") == ARTIFACT
              and a.get("expired") is False]
    require(len(native) == 1, "artefact natif unique, present et non expire requis")
    artifact = native[0]
    require(isinstance(artifact.get("size_in_bytes"), int)
            and artifact["size_in_bytes"] > 0,
            "archive native vide")
    require(isinstance(artifact.get("digest"), str)\n            and re.fullmatch(r"sha256:[0-9a-f]{64}", artifact["digest"]) is not None,
            "empreinte d'archive absente")
    if artifact.get("workflow_run") is not None:
        require(artifact["workflow_run"].get("id") == run["id"],
                "archive provenant d'un autre run")
    print("C22_NATIVE_ARTIFACT_SMOKE_ELIGIBLE "
          f"run={run['id']} sha={run['head_sha']} "
          f"non_qualifiant={int(run['conclusion'] != 'success')} "
          f"artifact_id={artifact['id']}")
    return True


def main(argv):
    if len(argv) != 6:
        raise SystemExit("usage: c22_native_preflight.py RUN_JSON JOBS_JSON ARTIFACTS_JSON RUN_ID REPOSITORY")
    run, jobs, artifacts = (json.loads(Path(path).read_text(encoding="utf-8"))
                            for path in argv[1:4])
    try:
        verify(run, jobs, artifacts, argv[4], argv[5])
    except (ValueError, TypeError, KeyError) as exc:
        print(f"::error::{exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
