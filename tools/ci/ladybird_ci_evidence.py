"""Audit GitHub dédié. Exige les verdicts existants et conserve les durées observées."""
import datetime
import json
import os
import time

from ladybird_empreinte import assure_commit, empreinte, empreinte_vulkan
from ladybird_provenance import gh_api, gh_declenche

DEPOT = os.environ["GITHUB_REPOSITORY"]
BRANCHE = os.environ["GITHUB_REF_NAME"]
API = gh_api(DEPOT)


def runs(workflow):
    return API(f"actions/workflows/{workflow}/runs?branch={BRANCHE}&per_page=30")["workflow_runs"]


def artefacts(run):
    return API(f"actions/runs/{run}/artifacts?per_page=100")["artifacts"]


def attend(filtre, secondes=10800):
    limite = time.monotonic() + secondes
    while time.monotonic() < limite:
        resultat = filtre()
        if resultat is not None:
            return resultat
        time.sleep(30)
    raise TimeoutError("Exécution GitHub attendue sans verdict dans le délai")


def jobs(run):
    return API(f"actions/runs/{run}/jobs?filter=latest&per_page=100")["jobs"]


def production_cpu(identite, reconstruire, attendue, succes=True):
    workflow = "ladybird-native-browser.yml"
    avant = {r["id"] for r in runs(workflow)}
    gh_declenche(DEPOT, BRANCHE, identite, reconstruire)()

    def nouveau():
        return next((r for r in runs(workflow) if r["id"] not in avant
                     and r["event"] == "workflow_dispatch"
                     and r["display_title"] == f"ladybird-artefact {identite}"), None)

    run_id = attend(nouveau, 600)["id"]
    print("CPU", run_id, identite)
    fin = attend(lambda: termine(run_id))
    assert fin["conclusion"] == ("success" if succes else "failure"), fin["conclusion"]
    liste = jobs(run_id)
    build = next(j for j in liste if j["name"] == "ladybird / build once")
    assert build["conclusion"] == attendue, build["conclusion"]
    if attendue == "success":
        assert {"bouchaud-ladybird-native-browser", "bouchaud-wpt-corpus"} <= {
            a["name"] for a in artefacts(run_id)}
    if not succes:
        assert not any(a["name"] == "bouchaud-ladybird-native-browser" for a in artefacts(run_id))
        assert any(j["name"] == "ladybird / provenance de l'artefact" and j["conclusion"] == "failure"
                   for j in liste)
    print(json.dumps({"type": "cpu", "run": run_id, "build": build["conclusion"],
                      "runner_build_s": duree(build)}))


def termine(run):
    etat = API(f"actions/runs/{run}")
    return etat if etat["status"] == "completed" else None


def duree(objet):
    if objet["conclusion"] == "skipped":
        return 0
    debut = datetime.datetime.fromisoformat(objet["started_at"].replace("Z", "+00:00"))
    fin = datetime.datetime.fromisoformat(objet["completed_at"].replace("Z", "+00:00"))
    return int((fin - debut).total_seconds())


def vulkan():
    workflow = "ladybird-vulkan-experimental.yml"
    identite = empreinte_vulkan()
    ids = os.environ.get("VALIDATION_VULKAN", "")
    if ids:
        historiques = [attend(lambda: termine(int(rid))) for rid in ids.split(",")]
    else:
        historiques = attend(lambda: liste_vulkan_terminee(workflow))
    vus = set()
    for run in historiques:
        run_id = run["id"]
        assert run["conclusion"] == "success", run_id
        assert run["path"] == ".github/workflows/ladybird-vulkan-experimental.yml", run_id
        assure_commit(run["head_sha"])
        assert empreinte_vulkan(run["head_sha"]) == identite, run_id
        build = next(j for j in jobs(run_id) if j["name"] == "build")
        etape = next(s for s in build["steps"] if s["name"] ==
                     "Construire le navigateur et son compositeur Vulkan")
        vus.add(etape["conclusion"])
        noms = {a["name"] for a in artefacts(run_id)}
        assert "bouchaud-ladybird-vulkan-experimental" in noms, run_id
        if etape["conclusion"] == "success":
            assert f"bouchaud-ladybird-vulkan-{identite}" in noms, run_id
        print(json.dumps({"type": "vulkan", "run": run_id, "empreinte": identite,
                          "build": etape["conclusion"], "runner_build_s": duree(etape),
                          "runner_job_s": duree(build)}))
    assert vus == {"success", "skipped"}, vus


def liste_vulkan_terminee(workflow):
    liste = runs(workflow)
    if len(liste) >= 2 and all(r["status"] == "completed" for r in liste[:2]):
        return liste[:2]
    return None


if __name__ == "__main__":
    if not os.environ.get("VALIDATION_VULKAN"):
        identite = empreinte()["empreinte"]
        production_cpu("0" * 64, False, "skipped", succes=False)
        production_cpu(identite, True, "success")
        production_cpu(identite, False, "skipped")
    vulkan()
