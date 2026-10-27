#!/usr/bin/env python3
"""Provenance de l'artefact Ladybird d'un run (BOUCHAUD_LADYBIRD_PROVENANCE_V1).

Decide, au debut d'un run ladybird-native-browser, si l'artefact doit etre
CONSTRUIT (2 h 15 a froid) ou REUTILISE depuis un run anterieur dont les
entrees de build sont identiques (tools/ci/ladybird_empreinte.py).

Les caches GitHub sont cloisonnes par branche et ne sont sauvegardes qu'a la
fin d'un build de deux heures : sur les runs observes, ccache restait a 0 % et
chaque branche neuve recompilait tout. Les ARTEFACTS, eux, sont lisibles depuis
n'importe quel run du depot pendant leur retention (14 jours). La reutilisation
passe donc par eux.

Regles (toutes doivent tenir, sinon on CONSTRUIT) :
  * palier `developpement` -- une qualification construit TOUJOURS ;
  * run source de CE depot, de CE workflow, evenement push ou
    workflow_dispatch (jamais pull_request : un fork peut publier un artefact
    du meme nom, c'est l'empoisonnement d'artefact classique) ;
  * job `ladybird / build once` de ce run conclu `success` (un artefact
    republie par une reutilisation n'est pas une source : on remonte toujours
    a un build reel) ;
  * artefacts `bouchaud-ladybird-native-browser` ET `bouchaud-wpt-corpus`
    presents et non expires dans ce run ;
  * empreinte(HEAD du run source) == empreinte(HEAD courant).

Le job de reutilisation reverifie ensuite le manifeste et la liaison par les
entrees (artifact_manifest.py + ladybird_empreinte.py lie) : la decision ne
suffit jamais a elle seule.

Dans le doute (API muette, reponse incomplete), on CONSTRUIT : le repli est
couteux, jamais faux.
"""
from __future__ import annotations

import json
import os
import subprocess
import sys

ARTEFACT = "bouchaud-ladybird-native-browser"
CORPUS = "bouchaud-wpt-corpus"
WORKFLOW = ".github/workflows/ladybird-native-browser.yml"
JOB_BUILD = "ladybird / build once"
EVENEMENTS_SURS = ("push", "workflow_dispatch")
PALIERS = ("developpement", "qualification")


def _champ(d, *cles):
    for cle in cles:
        if not isinstance(d, dict) or cle not in d:
            return None
        d = d[cle]
    return d


def candidats(artefacts: list) -> list[int]:
    """Runs portant un artefact Ladybird non expire, du plus recent au plus ancien."""
    vus, ordre = set(), []
    for a in sorted((a for a in artefacts if isinstance(a, dict)),
                    key=lambda a: a.get("id") or 0, reverse=True):
        rid = _champ(a, "workflow_run", "id")
        if a.get("name") != ARTEFACT or a.get("expired") is not False or not isinstance(rid, int):
            continue
        if rid not in vus:
            vus.add(rid)
            ordre.append(rid)
    return ordre


def run_eligible(run: dict, jobs: dict, artefacts_run: dict, depot: str) -> tuple[bool, str]:
    """(eligible, raison) pour un run source, sans regarder l'empreinte."""
    if _champ(run, "repository", "full_name") != depot or _champ(run, "head_repository", "full_name") != depot:
        return False, "depot etranger"
    if run.get("path") != WORKFLOW:
        return False, f"workflow {run.get('path')!r}"
    if run.get("event") not in EVENEMENTS_SURS:
        return False, f"evenement {run.get('event')!r}"
    if not isinstance(run.get("head_sha"), str) or len(run["head_sha"]) != 40:
        return False, "head_sha absent"
    builds = [j for j in (jobs.get("jobs") or []) if isinstance(j, dict) and j.get("name") == JOB_BUILD]
    if not builds or any(j.get("conclusion") != "success" for j in builds):
        return False, "build once absent ou non reussi"
    noms = {a.get("name") for a in (artefacts_run.get("artifacts") or [])
            if isinstance(a, dict) and a.get("expired") is False}
    for requis in (ARTEFACT, CORPUS):
        if requis not in noms:
            return False, f"artefact {requis} absent ou expire"
    return True, "ok"


def decide(palier, courant_id, empreinte_courante, liste_artefacts, api, empreinte_de, depot,
           limite=12, sortie=print):
    """{'mode': 'construit'|'reutilise', 'source_run': id|'', 'raison': str}."""
    if palier not in PALIERS:
        raise ValueError(f"palier inconnu {palier!r}")
    if palier == "qualification":
        return {"mode": "construit", "source_run": "", "raison": "qualification : build complet exige"}
    examines = 0
    for rid in candidats(liste_artefacts):
        if rid == courant_id:
            continue
        if examines >= limite:
            break
        examines += 1
        try:
            run = api(f"actions/runs/{rid}")
            ok, raison = run_eligible(run, api(f"actions/runs/{rid}/jobs?filter=latest&per_page=100"),
                                      api(f"actions/runs/{rid}/artifacts?per_page=100"), depot)
            if ok:
                empreinte = empreinte_de(run["head_sha"])
                ok = empreinte == empreinte_courante
                raison = "empreinte identique" if ok else f"empreinte {empreinte[:16]}"
        except Exception as exc:  # API, git : ce candidat ne prouve rien
            ok, raison = False, f"erreur {type(exc).__name__}: {exc}"
        sortie(f"PROVENANCE candidat run={rid} {'RETENU' if ok else 'ecarte'} ({raison})")
        if ok:
            return {"mode": "reutilise", "source_run": str(rid), "raison": f"run {rid}, {raison}"}
    return {"mode": "construit", "source_run": "", "raison": f"aucun run eligible parmi {examines}"}


def gh_api(depot):
    def appel(chemin):
        r = subprocess.run(["gh", "api", f"repos/{depot}/{chemin}"], check=True, capture_output=True,
                           text=True, timeout=60)
        return json.loads(r.stdout)
    return appel


def ecrit(cle, valeur):
    if os.environ.get("GITHUB_OUTPUT"):
        with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as f:
            f.write(f"{cle}={valeur}\n")


def main() -> int:
    sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
    import ladybird_empreinte as le

    env = os.environ
    palier, depot = env.get("PALIER", ""), env.get("DEPOT", "")
    courant = le.empreinte("HEAD")
    print(f"PROVENANCE palier={palier} empreinte={courant['empreinte']} head={courant['commit']}")

    def empreinte_de(sha):
        le.assure_commit(sha)
        return le.empreinte(sha)["empreinte"]

    try:
        if palier == "qualification":
            liste = []
        else:
            api = gh_api(depot)
            liste = _champ(api(f"actions/artifacts?name={ARTEFACT}&per_page=100"), "artifacts") or []
        d = decide(palier, int(env.get("RUN_ID") or 0), courant["empreinte"], liste,
                   gh_api(depot), empreinte_de, depot)
    except ValueError:
        raise
    except Exception as exc:
        d = {"mode": "construit", "source_run": "", "raison": f"API indisponible ({type(exc).__name__}) : repli build"}
    print(f"PROVENANCE_DECISION mode={d['mode']} source_run={d['source_run'] or '-'} raison={d['raison']}")
    ecrit("mode", d["mode"])
    ecrit("source_run", d["source_run"])
    ecrit("empreinte", courant["empreinte"])
    return 0


if __name__ == "__main__":
    sys.exit(main())
