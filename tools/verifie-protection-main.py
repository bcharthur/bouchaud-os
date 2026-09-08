#!/usr/bin/env python3
"""Les checks exiges par la protection existent-ils vraiment ?

La protection de `main` est le seul dispositif du projet qui rende
EXECUTOIRES les soixante-sept garde-fous. Elle a une facon particuliere de
casser : elle ne casse pas.

Si un des noms qu'elle exige cesse d'exister -- un job renomme, un workflow
scinde --, GitHub attend un verdict qui n'arrivera jamais. Toutes les PR
restent bloquees, personne ne comprend pourquoi, et le remede evident est de
retirer la protection. On perd alors tout, au lieu de corriger un nom.

Ce garde-fou verifie donc, SANS RESEAU, que chaque nom exige correspond a un
job qui existe, qui se declenche sur `pull_request`, et qui porte
`if: always()` -- sans quoi il ne rapporterait pas quand ses dependances sont
sautees, ce qui bloque tout aussi surement.

Avec un jeton, il regarde en plus si la protection est POSEE. Sans jeton, il
le dit et passe : une barriere qui echoue faute de reseau apprend a etre
ignoree.
"""

import json
import os
import re
import subprocess
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
WORKFLOWS = RACINE / ".github/workflows"
SCRIPT_SH = RACINE / "tools/ci/configure-protection.sh"
SCRIPT_PS = RACINE / "tools/ci/configure_protection.ps1"
DEPOT = "bcharthur/bouchaud-os"


def contextes(source, motif):
    bloc = re.search(motif, source, re.S)
    if bloc is None:
        return None
    return re.findall(r'"([A-Za-z0-9_\-/ ]+)"', bloc.group(1))


def jobs_des_workflows():
    """Chaque job : son nom rapporte, s'il se declenche sur PR, s'il est always()."""
    trouves = {}
    for chemin in sorted(WORKFLOWS.glob("*.yml")):
        source = chemin.read_text(encoding="utf-8")
        entete, _, corps = source.partition("\njobs:")
        if not corps:
            continue
        sur_pr = "pull_request" in entete
        # Les bornes viennent des DEBUTS DE JOB successifs. Chercher la
        # prochaine ligne commencant par deux espaces trouverait la premiere
        # cle INDENTEE du job lui-meme -- `    name:` commence bien par deux
        # espaces --, et le bloc examine serait vide.
        debuts = [m.start() for m in re.finditer(r"^  [A-Za-z0-9_-]+:\s*$", corps, re.M)]
        for index, m in enumerate(re.finditer(r"^  ([A-Za-z0-9_-]+):\s*$", corps, re.M)):
            debut = m.end()
            suivant = [d for d in debuts if d > m.start()]
            fin = suivant[0] if suivant else len(corps)
            bloc = corps[debut:fin]
            nom = re.search(r"^\s{4}name:\s*(.+)$", bloc, re.M)
            rapporte = nom.group(1).strip() if nom else m.group(1)
            trouves[rapporte] = {
                "workflow": chemin.name,
                "pull_request": sur_pr,
                "always": bool(re.search(r"^\s{4}if:\s*always\(\)", bloc, re.M)),
            }
    return trouves


def controle_en_ligne(fautes):
    jeton = os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN")
    if not jeton:
        print("protection main : aucun jeton, etat en ligne non consulte")
        return
    try:
        sortie = subprocess.run(
            [
                "curl", "-sS", "--max-time", "20",
                "-H", "Authorization: Bearer %s" % jeton,
                "-H", "Accept: application/vnd.github+json",
                "https://api.github.com/repos/%s/branches/main" % DEPOT,
            ],
            capture_output=True,
            text=True,
            timeout=30,
        )
        donnees = json.loads(sortie.stdout)
    except Exception:
        print("protection main : etat en ligne inconsultable, controle statique seul")
        return
    if not isinstance(donnees, dict) or "protected" not in donnees:
        print("protection main : reponse inattendue, controle statique seul")
        return
    if not donnees.get("protected"):
        # Ce n'est pas une faute du DEPOT : c'est un etat a corriger, et le
        # dire fort vaut mieux que faire echouer une barriere que personne ne
        # peut corriger depuis l'integration continue.
        print(
            "protection main : AVERTISSEMENT -- `main` n'est pas protege.\n"
            "  Les %d garde-fous du depot sont contournables par un push direct.\n"
            "  Remede : tools/ci/configure-protection.sh"
            % len(list((RACINE / "tools").rglob("verifie-*.py")))
        )
    else:
        print("protection main : `main` est protege")


def main():
    fautes = []
    for chemin in (SCRIPT_SH, SCRIPT_PS):
        if not chemin.exists():
            fautes.append("script absent : %s" % chemin.relative_to(RACINE).as_posix())
    if fautes:
        for faute in fautes:
            print("  - %s" % faute)
        return 1

    sh = SCRIPT_SH.read_text(encoding="utf-8")
    ps = SCRIPT_PS.read_text(encoding="utf-8")
    exiges_sh = contextes(sh, r'"contexts":\s*\[(.*?)\]')
    exiges_ps = contextes(ps, r"contexts\s*=\s*@\((.*?)\)")

    if not exiges_sh:
        fautes.append("configure-protection.sh : aucun check exige.")
    if not exiges_ps:
        fautes.append("configure_protection.ps1 : aucun check exige.")
    if exiges_sh and exiges_ps and sorted(exiges_sh) != sorted(exiges_ps):
        fautes.append(
            "les deux scripts de protection n'exigent pas les memes checks "
            "(%s contre %s) ; celui qu'on lancera decidera, et on ne saura pas "
            "lequel." % (sorted(exiges_sh), sorted(exiges_ps))
        )

    connus = jobs_des_workflows()
    for nom in exiges_sh or []:
        job = connus.get(nom)
        if job is None:
            fautes.append(
                "le check « %s » est exige par la protection et n'existe dans "
                "aucun workflow. GitHub attendrait un verdict qui n'arrive "
                "jamais : TOUTES les PR resteraient bloquees." % nom
            )
            continue
        if not job["pull_request"]:
            fautes.append(
                "le check « %s » (%s) ne se declenche pas sur pull_request ; "
                "il ne rapporterait jamais." % (nom, job["workflow"])
            )
        if not job["always"]:
            fautes.append(
                "le check « %s » (%s) n'est pas `if: always()` ; il serait "
                "saute des qu'une de ses dependances l'est, et la PR resterait "
                "bloquee." % (nom, job["workflow"])
            )

    if fautes:
        print("protection main : %d probleme(s)\n" % len(fautes))
        for faute in fautes:
            print("  - %s\n" % faute)
        return 1

    print(
        "protection main : les %d checks exiges existent, se declenchent sur "
        "pull_request et rapportent toujours" % len(exiges_sh)
    )
    controle_en_ligne(fautes)
    return 0


if __name__ == "__main__":
    sys.exit(main())
