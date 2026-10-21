#!/usr/bin/env python3
"""BOUCHAUD_CI_UN_BUILD_PAR_SHA_V2 : un build par arbre, sans jamais un faux vert.

Un push sur une branche `claude/**` ou `feat/**` qui porte une PR declenche
DEUX runs de chaque workflow : `push` (la tete) et `pull_request` (le commit de
fusion). Sur un arbre identique, le second refait le premier -- pour Ladybird,
des heures de build.

V1 sautait le run PR des qu'un run push EXISTAIT, meme en echec ou encore en
cours : une PR pouvait s'afficher verte sans qu'aucun test n'ait tourne ni
reussi. V2 separe la DECISION de la PREUVE :

  decide  (au debut du run PR) : reutiliser est PERMIS si TOUT tient --
          PR de ce depot, pas `labeled` ; arbre de fusion == arbre de tete ;
          un run `push` du meme workflow, sur la meme tete, qui est en cours
          ou termine en `success` / `failure`. Annule, saute, expire, neutre,
          perime : pas une preuve -> le run PR execute ses propres tests.
          Dans le doute (API muette, champ absent) : il les execute.

  attend  (job de verdict du run PR, `if: always()`, jamais saute) : relit le
          run source jusqu'a sa conclusion FINALE, puis exige run `success`
          ET job de verdict nomme `success` (ni saute, ni absent), meme tete,
          meme workflow, evenement `push`. Tout autre etat -- echec, annule,
          delai, API indisponible jusqu'au bout -- rend ROUGE. Un echec du
          push transmet donc un etat bloquant ; un run annule ne prouve rien.

Le verdict du run PR pointe ainsi vers une execution reelle, identifiee par son
run, sa tentative, son SHA, son arbre et son workflow.
"""
import argparse
import json
import os
import subprocess
import sys
import time

EN_COURS = ("queued", "in_progress", "waiting", "requested", "pending")
# Conclusions qui portent un resultat de tests : la reussite, et l'echec qui
# doit bloquer. Tout le reste n'est pas une preuve.
REUTILISABLES = ("success", "failure")


def _champ(d, *cles):
    for cle in cles:
        if not isinstance(d, dict) or cle not in d:
            return None
        d = d[cle]
    return d


def choisit_source(runs, tete, chemin_workflow):
    """Le run push le plus recent du meme workflow sur la meme tete, ou None."""
    candidats = [r for r in runs
                 if isinstance(r, dict) and r.get("event") == "push" and r.get("head_sha") == tete
                 and r.get("path") == chemin_workflow and isinstance(r.get("id"), int)]
    return max(candidats, key=lambda r: r["id"]) if candidats else None


def decide(evenement, action, depot, depot_tete, arbre_tete, arbre_fusion, source):
    """`(reutilise, raison)`. `source` : le run choisi par `choisit_source`."""
    if evenement != "pull_request":
        return False, f"evenement {evenement or '?'} : pas un run PR"
    if action == "labeled":
        return False, "libelle pose : demande explicite"
    if not depot or depot_tete != depot:
        return False, f"tete hors depot ({depot_tete or '?'})"
    if not arbre_tete or not arbre_fusion:
        return False, "arbre de tete ou de fusion inconnu"
    if arbre_tete != arbre_fusion:
        return False, f"fusion {arbre_fusion[:12]} != tete {arbre_tete[:12]} : la base a avance"
    if source is None:
        return False, "aucun run push du meme workflow sur la tete"
    statut, conclusion = source.get("status"), source.get("conclusion")
    if statut in EN_COURS:
        return True, f"run push {source['id']} en cours : le verdict l'attendra"
    if statut == "completed" and conclusion in REUTILISABLES:
        return True, f"run push {source['id']} termine en {conclusion} : le verdict le reflete"
    return False, f"run push {source['id']} {statut}/{conclusion} : pas une preuve, tests executes"


def juge_source(run, jobs, tete, chemin_workflow, job_verdict):
    """Run source TERMINE -> `(ok, code, message)`."""
    if _champ(run, "head_sha") != tete or _champ(run, "event") != "push" or _champ(run, "path") != chemin_workflow:
        return False, "SOURCE_INCOHERENTE", (f"run {_champ(run, 'id')} : sha={_champ(run, 'head_sha')} "
                                             f"event={_champ(run, 'event')} path={_champ(run, 'path')}")
    conclusion = _champ(run, "conclusion")
    if conclusion != "success":
        code = {"failure": "SOURCE_ECHEC", "cancelled": "SOURCE_ANNULEE", "skipped": "SOURCE_SAUTEE",
                "timed_out": "SOURCE_EXPIREE"}.get(conclusion, "SOURCE_NON_PROUVEE")
        suite = (" -- preuve absente : relancer ce workflow PR executera ses propres tests"
                 if code != "SOURCE_ECHEC" else " -- l'echec du push bloque la PR")
        return False, code, f"run {run['id']} conclusion={conclusion}{suite}"
    verdicts = [j for j in jobs if isinstance(j, dict) and j.get("name") == job_verdict]
    if not verdicts:
        return False, "VERDICT_ABSENT", f"run {run['id']} : aucun job `{job_verdict}`"
    dernier = max(verdicts, key=lambda j: j.get("id", 0))
    if dernier.get("status") != "completed" or dernier.get("conclusion") != "success":
        return False, "VERDICT_NON_PROUVE", (f"job `{job_verdict}` {dernier.get('status')}/"
                                             f"{dernier.get('conclusion')} (saute ou en echec : pas une preuve)")
    return True, "SOURCE_PROUVEE", f"run {run['id']} et job `{job_verdict}` ({dernier.get('id')}) en success"


def attend(api, dormir, maintenant, source_id, tete, chemin_workflow, job_verdict, delai_s, pas_s, sortie=print):
    """Attend la conclusion FINALE du run source puis le juge. `(ok, code, message)`."""
    limite = maintenant() + delai_s
    derniere_erreur = None
    while True:
        try:
            run = api(f"actions/runs/{source_id}")
            statut = _champ(run, "status")
            if statut == "completed":
                jobs, page = [], 1
                while True:
                    lot = api(f"actions/runs/{source_id}/jobs?per_page=100&page={page}")
                    morceau = _champ(lot, "jobs")
                    if not isinstance(morceau, list):
                        raise ValueError("reponse jobs incomplete")
                    jobs += morceau
                    if len(morceau) < 100:
                        break
                    page += 1
                    if page > 20:  # une API qui ne s'arrete pas n'est pas une reponse
                        raise ValueError("pagination des jobs sans fin")
                return juge_source(run, jobs, tete, chemin_workflow, job_verdict)
            if statut not in EN_COURS:
                raise ValueError(f"statut inconnu {statut!r}")
            sortie(f"ATTEND run={source_id} statut={statut} tentative={_champ(run, 'run_attempt')}")
        except Exception as exc:  # reseau, API, reponse incomplete : on reessaie
            derniere_erreur = f"{type(exc).__name__}: {exc}"
            sortie(f"ATTEND run={source_id} erreur={derniere_erreur}")
        if maintenant() >= limite:
            if derniere_erreur:
                return False, "API_INDISPONIBLE", f"delai epuise, derniere erreur : {derniere_erreur}"
            return False, "DELAI", f"run {source_id} non termine apres {delai_s} s"
        dormir(pas_s)


def gh_api(depot):
    def appel(chemin):
        r = subprocess.run(["gh", "api", f"repos/{depot}/{chemin}"], check=True, capture_output=True,
                           text=True, timeout=60)
        return json.loads(r.stdout)
    return appel


def chemin_du_workflow(workflow_ref):
    # "proprietaire/depot/.github/workflows/ci.yml@refs/pull/226/merge"
    sans_ref = workflow_ref.split("@", 1)[0]
    i = sans_ref.find("/.github/")
    return sans_ref[i + 1:] if i >= 0 else ""


def ecrit(cle, valeur):
    if os.environ.get(cle):
        with open(os.environ[cle], "a", encoding="utf-8") as f:
            f.write(valeur + "\n")


def main_decide():
    env = os.environ
    evenement, depot = env.get("EVENEMENT", ""), env.get("DEPOT", "")
    source = None
    try:
        if evenement != "pull_request":
            reutilise, raison = decide(evenement, "", "", "", "", "", None)
        else:
            api = gh_api(depot)
            tete, fusion = env["TETE"], env["FUSION"]
            chemin = chemin_du_workflow(env["WORKFLOW_REF"])
            arbre_tete = _champ(api(f"git/commits/{tete}"), "tree", "sha")
            arbre_fusion = _champ(api(f"git/commits/{fusion}"), "tree", "sha")
            fichier = chemin.rsplit("/", 1)[-1]
            runs = _champ(api(f"actions/workflows/{fichier}/runs?event=push&head_sha={tete}&per_page=50"),
                          "workflow_runs") or []
            source = choisit_source(runs, tete, chemin)
            reutilise, raison = decide(evenement, env.get("ACTION", ""), depot, env.get("DEPOT_TETE", ""),
                                       arbre_tete, arbre_fusion, source)
    except Exception as exc:  # repli : on teste
        reutilise, raison = False, f"decision impossible ({type(exc).__name__}: {exc}) : tests executes"
    ligne = f"DEDOUBLONNE reutilise={'true' if reutilise else 'false'} raison={raison}"
    print(ligne)
    ecrit("GITHUB_OUTPUT", f"reutilise={'true' if reutilise else 'false'}")
    ecrit("GITHUB_OUTPUT", f"source_run={source['id'] if (reutilise and source) else ''}")
    ecrit("GITHUB_STEP_SUMMARY", ligne)
    return 0


def main_attend(args):
    env = os.environ
    depot, tete = env.get("DEPOT", ""), env.get("TETE", "")
    chemin = chemin_du_workflow(env.get("WORKFLOW_REF", ""))
    if not (args.run.isdigit() and depot and tete and chemin):
        print(f"REUTILISATION_ECHEC code=PARAMETRES run={args.run!r} tete={tete!r} workflow={chemin!r}")
        return 1
    ok, code, message = attend(gh_api(depot), time.sleep, time.monotonic, int(args.run), tete, chemin,
                               args.job, args.delai, args.pas)
    try:
        arbre = _champ(gh_api(depot)(f"git/commits/{tete}"), "tree", "sha")
    except Exception:
        arbre = "?"
    ligne = (f"REUTILISATION_{'OK' if ok else 'ECHEC'} code={code} source=run:{args.run} sha={tete} "
             f"arbre={arbre} workflow={chemin} -- {message}")
    print(ligne)
    ecrit("GITHUB_STEP_SUMMARY", ligne)
    return 0 if ok else 1


def main():
    if len(sys.argv) > 1 and sys.argv[1] == "attend":
        p = argparse.ArgumentParser()
        p.add_argument("mode")
        p.add_argument("--run", required=True)
        p.add_argument("--job", required=True)
        p.add_argument("--delai", type=int, default=5 * 3600)
        p.add_argument("--pas", type=int, default=60)
        return main_attend(p.parse_args())
    return main_decide()


if __name__ == "__main__":
    sys.exit(main())
