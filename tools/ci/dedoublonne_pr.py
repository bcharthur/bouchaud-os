#!/usr/bin/env python3
"""BOUCHAUD_CI_UN_BUILD_PAR_SHA_V1 : le run `pull_request` refait-il un run `push` ?

Un push sur une branche `claude/**` ou `feat/**` qui porte une PR declenche
DEUX runs de chaque workflow : `push`, qui teste la tete, et `pull_request`,
qui teste le commit de fusion tete + base. Pour Ladybird, c'est deux builds de
plusieurs heures sur le meme arbre (ed5602f4 : runs 39479633757 et
39479638849, memes sondes, memes images).

Ils ne sont redondants que si TOUT ceci tient :
  * l'evenement est `pull_request` depuis CE depot (une PR de fork n'a pas de
    run push ici) et ce n'est pas une demande explicite (`labeled`) ;
  * le run `push` du MEME workflow existe sur la MEME tete et n'est ni annule
    ni saute (les filtres de chemins different d'un declencheur a l'autre :
    integration.yml filtre le push, pas la PR) ;
  * l'arbre du commit de fusion est IDENTIQUE a celui de la tete -- la base
    est deja contenue. Si `main` a avance, la fusion est un autre arbre, et
    le run PR est une verification a part entiere : il tourne.

Dans le doute -- API muette, champ absent, erreur --, le run PR tourne. Un
doublon de trop coute des minutes ; une verification sautee a tort coute une
preuve.
"""
import json
import os
import subprocess
import sys

RUNS_ECARTES = ("cancelled", "skipped")


def decide(evenement, action, depot, depot_tete, arbre_tete, arbre_fusion, runs_push):
    """`(doublon, raison)`. `runs_push` : les runs `push` du meme workflow sur
    la meme tete (dicts de l'API : `conclusion`, `html_url`)."""
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
    vivants = [r for r in runs_push if r.get("conclusion") not in RUNS_ECARTES]
    if not vivants:
        return False, f"aucun run push vivant sur la tete ({len(runs_push)} ecarte(s))"
    return True, f"meme arbre {arbre_tete[:12]} deja teste par {vivants[0].get('html_url', '?')}"


def gh_json(chemin):
    sortie = subprocess.run(["gh", "api", chemin], check=True, capture_output=True, text=True, timeout=60)
    return json.loads(sortie.stdout)


def fichier_workflow(workflow_ref):
    # "proprietaire/depot/.github/workflows/ci.yml@refs/pull/226/merge"
    return workflow_ref.split("@", 1)[0].rsplit("/", 1)[-1]


def main():
    env = os.environ
    evenement = env.get("EVENEMENT", "")
    depot = env.get("DEPOT", "")
    try:
        if evenement != "pull_request":
            doublon, raison = decide(evenement, "", "", "", "", "", [])
        else:
            tete = env["TETE"]
            fusion = env["FUSION"]
            arbre_tete = gh_json(f"repos/{depot}/git/commits/{tete}")["tree"]["sha"]
            arbre_fusion = gh_json(f"repos/{depot}/git/commits/{fusion}")["tree"]["sha"]
            fichier = fichier_workflow(env["WORKFLOW_REF"])
            runs = gh_json(
                f"repos/{depot}/actions/workflows/{fichier}/runs?event=push&head_sha={tete}&per_page=20"
            ).get("workflow_runs", [])
            doublon, raison = decide(
                evenement, env.get("ACTION", ""), depot, env.get("DEPOT_TETE", ""),
                arbre_tete, arbre_fusion, runs,
            )
    except Exception as exc:  # repli : on teste
        doublon, raison = False, f"decision impossible ({type(exc).__name__}: {exc}) : le run tourne"
    ligne = f"DEDOUBLONNE doublon={'true' if doublon else 'false'} raison={raison}"
    print(ligne)
    for cle, valeur in (("GITHUB_OUTPUT", f"doublon={'true' if doublon else 'false'}"),
                        ("GITHUB_STEP_SUMMARY", ligne)):
        if env.get(cle):
            with open(env[cle], "a", encoding="utf-8") as f:
                f.write(valeur + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
