#!/usr/bin/env python3
"""Provenance de l'artefact Ladybird d'un run (BOUCHAUD_LADYBIRD_PROVENANCE_V2).

Decide, au debut d'un run ladybird-native-browser, d'ou vient l'artefact
Ladybird que ce run va jouer. Le navigateur ne depend PAS du noyau : il
depend de ses entrees de build (tools/ci/ladybird_empreinte.py). Une
qualification du noyau n'a donc aucune raison de recompiler 2 h 20 un
navigateur dont les entrees n'ont pas change (mesure : l'empreinte 6199f460
a ete compilee trois fois en entier, 99, 143 et 144 min, du 9 au 10/11).

Modes rendus :
  reutilise  un artefact de MEME empreinte, issu d'un build reel, existe :
             il est telecharge puis REVERIFIE (manifeste + liaison par les
             entrees) avant d'etre republie dans ce run ;
  attendu    aucun artefact, mais une PRODUCTION de cette empreinte est en
             cours (ou vient d'etre declenchee par ce run) : le job `attente`
             l'attend, puis la reutilisation la reverifie ;
  construit  repli : build complet dans ce run (production impossible a
             declencher, p. ex. pull_request d'un fork) ;
  existe     palier `artefact` seulement : l'artefact existe deja, la
             production n'a rien a faire.

La PRODUCTION est un run de ce meme workflow, palier `artefact`, declenche par
workflow_dispatch. Elle a son propre groupe de concurrence, indexe par
l'empreinte et SANS annulation : un nouveau push noyau annule le run de
qualification de sa branche, jamais un build Ladybird encore utile ; deux
productions de la meme empreinte se succedent, la seconde trouve l'artefact
et ne compile rien.

Regles d'eligibilite d'un run source (toutes, sinon il est ecarte) :
  * run de CE depot, de CE workflow, evenement push ou workflow_dispatch
    (jamais pull_request : un fork peut publier un artefact du meme nom) ;
  * job `ladybird / build once` de ce run conclu `success` (un artefact
    republie par une reutilisation n'est pas une source) ;
  * artefacts `bouchaud-ladybird-native-browser` ET `bouchaud-wpt-corpus`
    presents et non expires ;
  * empreinte(head_sha du run source), recalculee depuis git, egale a
    l'empreinte courante -- jamais lue dans un titre ou un nom d'artefact ;
  * avec `reconstruire` : run source cree APRES le debut de ce run (audit :
    un build neuf est exige, aucun artefact anterieur n'est accepte).

Dans le doute (API muette, reponse incomplete), on ne reutilise pas.
"""
from __future__ import annotations

import json
import os
import subprocess
import sys
import time

ARTEFACT = "bouchaud-ladybird-native-browser"
CORPUS = "bouchaud-wpt-corpus"
WORKFLOW = ".github/workflows/ladybird-native-browser.yml"
FICHIER_WORKFLOW = "ladybird-native-browser.yml"
JOB_BUILD = "ladybird / build once"
EVENEMENTS_SURS = ("push", "workflow_dispatch")
CONSOMMATEURS = ("developpement", "qualification")
PRODUCTION = "artefact"
PALIERS = CONSOMMATEURS + (PRODUCTION,)
# Titre (run-name) des runs de production : sert a les TROUVER, jamais a les
# croire -- l'empreinte est toujours recalculee depuis leur head_sha.
TITRE_PRODUCTION = "ladybird-artefact "
EN_COURS = ("in_progress", "queued", "pending", "waiting", "requested")


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


def run_sur(run: dict, depot: str) -> tuple[bool, str]:
    """Le run vient-il de ce depot, de ce workflow, d'un evenement sur ?"""
    if _champ(run, "repository", "full_name") != depot or _champ(run, "head_repository", "full_name") != depot:
        return False, "depot etranger"
    if run.get("path") != WORKFLOW:
        return False, f"workflow {run.get('path')!r}"
    if run.get("event") not in EVENEMENTS_SURS:
        return False, f"evenement {run.get('event')!r}"
    if not isinstance(run.get("head_sha"), str) or len(run["head_sha"]) != 40:
        return False, "head_sha absent"
    return True, "ok"


def run_eligible(run: dict, jobs: dict, artefacts_run: dict, depot: str) -> tuple[bool, str]:
    """(eligible, raison) pour un run source, sans regarder l'empreinte."""
    ok, raison = run_sur(run, depot)
    if not ok:
        return ok, raison
    builds = [j for j in (jobs.get("jobs") or []) if isinstance(j, dict) and j.get("name") == JOB_BUILD]
    if not builds or any(j.get("conclusion") != "success" for j in builds):
        return False, "build once absent ou non reussi"
    noms = {a.get("name") for a in (artefacts_run.get("artifacts") or [])
            if isinstance(a, dict) and a.get("expired") is False}
    for requis in (ARTEFACT, CORPUS):
        if requis not in noms:
            return False, f"artefact {requis} absent ou expire"
    return True, "ok"


class Empreintes:
    """empreinte(head_sha), memorisee : plusieurs runs partagent un commit."""

    def __init__(self, calcul):
        self.calcul, self.vues = calcul, {}

    def __call__(self, sha):
        if sha not in self.vues:
            self.vues[sha] = self.calcul(sha)
        return self.vues[sha]


def cherche_artefact(courant_id, empreinte_courante, liste_artefacts, api, empreinte_de, depot,
                     depuis=None, limite=40, sortie=print):
    """(run_id, raison) du run source le plus recent eligible, ou (None, raison).

    L'empreinte (calcul local, sans API) est comparee AVANT de lire jobs et
    artefacts : un run d'une autre recette coute un appel, pas trois.
    """
    examines = 0
    for rid in candidats(liste_artefacts):
        if rid == courant_id:
            continue
        if examines >= limite:
            break
        examines += 1
        try:
            ok, raison = examine(rid, empreinte_courante, api, empreinte_de, depot, depuis)
        except Exception as exc:  # API, git : ce candidat ne prouve rien
            ok, raison = False, f"erreur {type(exc).__name__}: {exc}"
        sortie(f"PROVENANCE candidat run={rid} {'RETENU' if ok else 'ecarte'} ({raison})")
        if ok:
            return rid, raison
    return None, f"aucun run eligible parmi {examines}"


def productions_en_cours(courant_id, empreinte_courante, api, empreinte_de, depot, sortie=print):
    """Runs de production (palier artefact) en cours pour CETTE empreinte."""
    trouves = []
    for statut in EN_COURS:
        reponse = api(f"actions/workflows/{FICHIER_WORKFLOW}/runs?event=workflow_dispatch&status={statut}&per_page=50")
        for run in (reponse.get("workflow_runs") or []):
            rid = run.get("id")
            if not isinstance(rid, int) or rid == courant_id or rid in trouves:
                continue
            if not str(run.get("display_title") or "").startswith(TITRE_PRODUCTION):
                continue
            ok, raison = run_sur(run, depot)
            if ok:
                ok = empreinte_de(run["head_sha"]) == empreinte_courante
                raison = "meme empreinte" if ok else "autre empreinte"
            sortie(f"PROVENANCE production run={rid} statut={statut} {'SUIVIE' if ok else 'ignoree'} ({raison})")
            if ok:
                trouves.append(rid)
    return trouves


def decide(palier, courant_id, empreinte_courante, liste_artefacts, api, empreinte_de, depot,
           reconstruire=False, depuis=None, declenche=None, sortie=print):
    """{'mode', 'source_run', 'raison'} ; voir l'en-tete du module."""
    if palier not in PALIERS:
        raise ValueError(f"palier inconnu {palier!r}")
    empreinte_de = Empreintes(empreinte_de)
    if not reconstruire:
        rid, raison = cherche_artefact(courant_id, empreinte_courante, liste_artefacts, api, empreinte_de,
                                       depot, sortie=sortie)
        if rid is not None:
            mode = "existe" if palier == PRODUCTION else "reutilise"
            return {"mode": mode, "source_run": str(rid), "raison": f"run {rid}, {raison}"}
    else:
        raison = "reconstruction exigee"
    if palier == PRODUCTION:
        # La concurrence (groupe par empreinte, sans annulation) a deja
        # serialise les productions de cette empreinte : on construit.
        return {"mode": "construit", "source_run": "", "raison": raison}
    if not reconstruire:
        en_cours = productions_en_cours(courant_id, empreinte_courante, api, empreinte_de, depot, sortie)
        if en_cours:
            return {"mode": "attendu", "source_run": "",
                    "raison": f"{raison} ; production en cours run={en_cours[0]}"}
    if declenche is not None:
        try:
            declenche()
            return {"mode": "attendu", "source_run": "", "raison": f"{raison} ; production declenchee"}
        except Exception as exc:
            raison = f"{raison} ; production impossible ({type(exc).__name__}: {exc})"
    return {"mode": "construit", "source_run": "", "raison": f"{raison} : repli build dans le run"}


class AttenteEchec(RuntimeError):
    """Fin d'attente sans artefact : plus de production, ou delai depasse."""


def examine(rid, empreinte_courante, api, empreinte_de, depot, depuis=None):
    """(eligible, raison) d'un run source precis : regles completes, empreinte comprise."""
    run = api(f"actions/runs/{rid}")
    ok, raison = run_sur(run, depot)
    if ok and depuis and str(run.get("created_at") or "") < depuis:
        return False, f"anterieur a {depuis} (reconstruction exigee)"
    if ok:
        empreinte = empreinte_de(run["head_sha"])
        if empreinte != empreinte_courante:
            return False, f"empreinte {empreinte[:16]}"
        ok, raison = run_eligible(run, api(f"actions/runs/{rid}/jobs?filter=latest&per_page=100"),
                                  api(f"actions/runs/{rid}/artifacts?per_page=100"), depot)
        raison = "empreinte identique, build reel, artefacts presents" if ok else raison
    return ok, raison


def attends(courant_id, empreinte_courante, api, liste, empreinte_de, depot, depuis=None,
            delai=14400, pas=60, grace=300, rafraichit=10, horloge=time.monotonic, dort=time.sleep,
            sortie=print):
    """Attend un artefact eligible de cette empreinte ; (run_id, raison) ou exception.

    Budget d'API borne (revue de la PR #282 : le jeton du job est limite a
    1 000 requetes/heure/depot) : UNE recherche complete au depart ; ensuite,
    a chaque tour, une requete d'etat par production suivie, et la liste des
    productions en cours n'est relue que tant qu'aucune n'est suivie, ou un
    tour sur `rafraichit`. Une production terminee en succes est revérifiee
    en entier (`examine`) ; terminee autrement, elle cesse d'etre suivie.
    Une erreur d'API (limite de debit comprise) fait patienter, pas echouer ;
    seul le delai borne l'attente. Plus aucune production suivie apres la
    grace : une derniere recherche complete, puis echec.
    """
    empreinte_de = Empreintes(empreinte_de)
    debut = horloge()
    rid, raison = cherche_artefact(courant_id, empreinte_courante, liste(), api, empreinte_de, depot,
                                   depuis=depuis, sortie=sortie)
    if rid is not None:
        return rid, raison
    suivies, tour = [], 0
    while True:
        ecoule = horloge() - debut
        try:
            if not suivies or tour % rafraichit == 0:
                for pid in productions_en_cours(courant_id, empreinte_courante, api, empreinte_de, depot,
                                                sortie):
                    if pid not in suivies:
                        suivies.append(pid)
            for pid in list(suivies):
                run = api(f"actions/runs/{pid}")
                if run.get("status") != "completed":
                    continue
                suivies.remove(pid)
                if run.get("conclusion") != "success":
                    sortie(f"PROVENANCE production run={pid} terminee : {run.get('conclusion')}")
                    continue
                ok, raison = examine(pid, empreinte_courante, api, empreinte_de, depot, depuis)
                sortie(f"PROVENANCE production run={pid} terminee : {'RETENUE' if ok else 'ecartee'} ({raison})")
                if ok:
                    return pid, raison
            if not suivies and ecoule >= grace:
                rid, raison = cherche_artefact(courant_id, empreinte_courante, liste(), api, empreinte_de,
                                               depot, depuis=depuis, sortie=sortie)
                if rid is not None:
                    return rid, raison
                raise AttenteEchec(f"aucune production en cours et aucun artefact ({raison})")
        except AttenteEchec:
            raise
        except Exception as exc:  # API momentanement indisponible ou limite de debit
            sortie(f"PROVENANCE attente : erreur d'API ({type(exc).__name__}: {exc}), nouvel essai")
        if ecoule >= delai:
            raise AttenteEchec(f"delai {delai} s depasse ({raison})")
        tour += 1
        dort(pas)


def gh_api(depot):
    def appel(chemin):
        r = subprocess.run(["gh", "api", f"repos/{depot}/{chemin}"], check=True, capture_output=True,
                           text=True, timeout=60)
        return json.loads(r.stdout)
    return appel


def gh_declenche(depot, ref, empreinte, reconstruire):
    def declenche():
        subprocess.run(["gh", "api", "-X", "POST", f"repos/{depot}/actions/workflows/{FICHIER_WORKFLOW}/dispatches",
                        "-f", f"ref={ref}", "-f", f"inputs[palier]={PRODUCTION}", "-f", f"inputs[empreinte]={empreinte}",
                        "-f", f"inputs[reconstruire]={'true' if reconstruire else 'false'}"],
                       check=True, capture_output=True, text=True, timeout=60)
        print(f"PROVENANCE production declenchee ref={ref} empreinte={empreinte} reconstruire={reconstruire}")
    return declenche


def ecrit(cle, valeur):
    if os.environ.get("GITHUB_OUTPUT"):
        with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as f:
            f.write(f"{cle}={valeur}\n")


def liste_artefacts(api):
    return _champ(api(f"actions/artifacts?name={ARTEFACT}&per_page=100"), "artifacts") or []


def verdict_production(env) -> tuple[bool, str]:
    """Palier artefact : provenance jouee, build reussi ou artefact deja present."""
    prov, build, art, mode = (env.get(k) or "absent" for k in ("P_PROVENANCE", "P_BUILD", "P_ARTEFACT", "P_MODE"))
    if prov != "success":
        return False, f"provenance={prov}"
    if art != "skipped":
        return False, f"rendez-vous artefact={art} (aucune epreuve en production)"
    if mode == "construit" and build == "success":
        return True, "artefact construit et publie"
    if mode == "existe" and build == "skipped":
        return True, "artefact deja present, rien a construire"
    return False, f"mode={mode} build={build}"


def main(argv=None) -> int:
    sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
    argv = sys.argv[1:] if argv is None else argv
    env = os.environ
    if argv[:1] == ["verdict-production"]:
        ok, raison = verdict_production(env)
        print(f"LADYBIRD_PRODUCTION_{'OK' if ok else 'ECHEC'} {raison}")
        return 0 if ok else 1
    import ladybird_empreinte as le

    palier, depot = env.get("PALIER", ""), env.get("DEPOT", "")
    reconstruire = env.get("RECONSTRUIRE", "") == "true"
    courant_id = int(env.get("RUN_ID") or 0)
    courant = le.empreinte("HEAD")
    print(f"PROVENANCE palier={palier} reconstruire={reconstruire} empreinte={courant['empreinte']} "
          f"head={courant['commit']}")

    def empreinte_de(sha):
        le.assure_commit(sha)
        return le.empreinte(sha)["empreinte"]

    api = gh_api(depot)
    # `depuis` : debut de CE run ; une reconstruction n'accepte qu'un build posterieur.
    depuis = None
    if reconstruire:
        depuis = str(api(f"actions/runs/{courant_id}").get("run_started_at") or "") or None

    if argv[:1] == ["attends"]:
        try:
            rid, raison = attends(courant_id, courant["empreinte"], api, lambda: liste_artefacts(api),
                                  empreinte_de, depot, depuis=depuis,
                                  delai=int(env.get("DELAI_ATTENTE") or 14400))
        except Exception as exc:
            print(f"LADYBIRD_ATTENTE_ECHEC {exc}")
            return 1
        print(f"LADYBIRD_ATTENTE_OK source_run={rid} ({raison})")
        ecrit("source_run", str(rid))
        return 0

    if palier == PRODUCTION:
        demandee = env.get("EMPREINTE_DEMANDEE", "")
        if demandee != courant["empreinte"]:
            # La branche a change de recette depuis le declenchement : ce run
            # produirait une autre empreinte que celle que l'on attend de lui.
            print(f"LADYBIRD_PRODUCTION_ECHEC empreinte demandee {demandee[:16] or '(vide)'} "
                  f"!= tete {courant['empreinte'][:16]}")
            return 1

    declenche = None
    if palier in CONSOMMATEURS and env.get("REF_PRODUCTION"):
        declenche = gh_declenche(depot, env["REF_PRODUCTION"], courant["empreinte"], reconstruire)
    try:
        d = decide(palier, courant_id, courant["empreinte"], liste_artefacts(api), api, empreinte_de, depot,
                   reconstruire=reconstruire, depuis=depuis, declenche=declenche)
    except ValueError:
        raise
    except Exception as exc:
        d = {"mode": "construit", "source_run": "",
             "raison": f"API indisponible ({type(exc).__name__}) : repli build"}
    print(f"PROVENANCE_DECISION mode={d['mode']} source_run={d['source_run'] or '-'} raison={d['raison']}")
    ecrit("mode", d["mode"])
    ecrit("source_run", d["source_run"])
    ecrit("empreinte", courant["empreinte"])
    return 0


if __name__ == "__main__":
    sys.exit(main())
