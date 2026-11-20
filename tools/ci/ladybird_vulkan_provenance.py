#!/usr/bin/env python3
"""Retrouve une archive Vulkan publiée par un run fiable aux mêmes entrées."""

import os
import sys

from ladybird_empreinte import assure_commit, empreinte_vulkan
from ladybird_provenance import ecrit, gh_api

WORKFLOW = ".github/workflows/ladybird-vulkan-experimental.yml"
PREFIXE = "bouchaud-ladybird-vulkan-"


def source(api, depot, empreinte, empreinte_de):
    nom = PREFIXE + empreinte
    for artefact in api(f"actions/artifacts?name={nom}&per_page=100")["artifacts"]:
        lien = artefact.get("workflow_run") or {}
        run_id = lien.get("id")
        if artefact["name"] != nom or artefact["expired"] or not isinstance(run_id, int):
            continue
        run = api(f"actions/runs/{run_id}")
        if (run["path"] != WORKFLOW or run["event"] != "push" or run["conclusion"] != "success"
                or run["repository"]["full_name"] != depot or run["head_repository"]["full_name"] != depot):
            continue
        if empreinte_de(run["head_sha"]) == empreinte:
            return run_id
    return None


def calcule(sha):
    assure_commit(sha)
    return empreinte_vulkan(sha)


def main():
    empreinte = empreinte_vulkan()
    if sys.argv[1:] == ["calcule"]:
        ecrit("empreinte", empreinte)
        return
    if empreinte != os.environ["EMPREINTE"]:
        raise SystemExit("LADYBIRD_VULKAN_PROVENANCE_ECHEC checkout incohérent")
    depot = os.environ["DEPOT"]
    run_id = source(gh_api(depot), depot, empreinte, calcule)
    ecrit("source_run", str(run_id) if run_id else "")


if __name__ == "__main__":
    main()
