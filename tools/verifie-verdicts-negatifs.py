#!/usr/bin/env python3
"""BOUCHAUD_CI_VERDICTS_NEGATIFS_V1 : un verdict negatif rend la CI rouge.

Cause : au run 42842860256 (SHA 93d9421), `python3 bench_ladybird.py ... |
tee -a "$GITHUB_STEP_SUMMARY"` a imprime LADYBIRD_BENCH_ECHEC et l'etape est
restee VERTE : le shell par defaut d'une etape (`bash -e`, sans pipefail)
rend le statut de `tee`, pas celui du verificateur.

Regles, sur chaque etape de chaque workflow :
  A. un tube vers `tee` dont la partie gauche execute autre chose que
     echo/printf exige pipefail (`shell: bash`, defaut bash du job ou du
     workflow, ou `set -o pipefail` / `set -euo pipefail` dans l'etape) ;
  B. aucun `|| true` / `|| :` derriere un verificateur (verifie, verdict,
     bench, analyse, qualifie, juge, preuve, check) ;
  C. `continue-on-error` seulement aux emplacements recenses ci-dessous,
     chacun diagnostic et justifie ; tout nouvel emplacement est rouge.

Fail-closed ; chaque regle a ses temoins negatifs.
"""
import re
import sys
from pathlib import Path

import yaml

RACINE = Path(__file__).resolve().parents[1]

# (fichier, job, etape ou None pour le job) -> justification. Aucun ne porte
# un verdict exige par une qualification.
TOLERES = {
    ("integration.yml", "qt-pixel", None): "rendu Qt hote, diagnostic sans porte",
    ("ladybird-sync.yml", "upstream-head", None): "information sur la tete upstream, reseau externe",
    ("native-ipc-runtime.yml", "ab-rollback", "Chronologie USB/SMP/A-B par boot (rapport facultatif)"):
        "rapport facultatif, ne modifie aucun verdict A/B",
    ("nightly.yml", "windows-debt", "ASCII strict global"): "dette Windows mesuree la nuit",
    ("nightly.yml", "windows-debt", "Syntaxe PowerShell globale"): "dette Windows mesuree la nuit",
    ("os-primitives.yml", "gpu-tampon",
     "P10+.3c banc de dechirure (contrat a deux tampons, TCG + KVM, SMP2 ; diagnostic, gele)"):
        "P10+.3c gele, offre OFF par defaut ; redeviendra bloquant avant tout changement de defaut",
}
VERIFICATEUR = re.compile(r"(verifie|verdict|bench|analyse|qualifie|juge|preuve|check)", re.I)
PIPEFAIL = re.compile(r"set -[a-z]*o pipefail|set -o pipefail")
INOFFENSIF = re.compile(r"^\s*(?:for [^;]+; do\s+)?(?:echo|printf)\b")


def logique(run: str) -> list:
    """Lignes logiques : continuations `\\` jointes, commentaires retires."""
    lignes, courante = [], ""
    for l in run.split("\n"):
        code = l.split(" #", 1)[0] if not l.lstrip().startswith("#") else ""
        if code.rstrip().endswith("\\"):
            courante += code.rstrip()[:-1] + " "
            continue
        lignes.append(courante + code)
        courante = ""
    if courante:
        lignes.append(courante)
    return lignes


def verifie_workflow(nom: str, texte: str) -> list:
    fautes = []
    try:
        w = yaml.safe_load(texte) or {}
    except yaml.YAMLError as exc:
        return [f"{nom} : YAML illisible ({exc})"]
    defaut = ((w.get("defaults") or {}).get("run") or {}).get("shell")
    for jid, job in (w.get("jobs") or {}).items():
        if not isinstance(job, dict):
            continue
        if job.get("continue-on-error") not in (None, False) and (nom, jid, None) not in TOLERES:
            fautes.append(f"{nom}:{jid} : continue-on-error non recense")
        shell_job = ((job.get("defaults") or {}).get("run") or {}).get("shell") or defaut
        for etape in job.get("steps") or []:
            nom_etape = etape.get("name") or etape.get("uses") or "?"
            if etape.get("continue-on-error") not in (None, False) and (nom, jid, nom_etape) not in TOLERES:
                fautes.append(f"{nom}:{jid}:{nom_etape} : continue-on-error non recense")
            run = etape.get("run") or ""
            shell = etape.get("shell") or shell_job
            pipefail = shell == "bash" or bool(PIPEFAIL.search(run))
            for ligne in logique(run):
                m = re.search(r"^(.*?)\|\s*tee\b", ligne)
                if m and not pipefail and not INOFFENSIF.match(m.group(1)):
                    fautes.append(f"{nom}:{jid}:{nom_etape} : `| tee` sans pipefail masque le statut de "
                                  f"`{m.group(1).strip()[:60]}`")
                m = re.search(r"^(.*)\|\|\s*(?:true|:)\s*$", ligne)
                if m and VERIFICATEUR.search(m.group(1)):
                    fautes.append(f"{nom}:{jid}:{nom_etape} : `|| true` derriere un verificateur "
                                  f"`{m.group(1).strip()[:60]}`")
    return fautes


def verifie(racine: Path) -> list:
    fautes = []
    for f in sorted((racine / ".github/workflows").glob("*.yml")):
        fautes += verifie_workflow(f.name, f.read_text(encoding="utf-8"))
    vus = set()
    for f in sorted((racine / ".github/workflows").glob("*.yml")):
        w = yaml.safe_load(f.read_text(encoding="utf-8")) or {}
        for jid, job in (w.get("jobs") or {}).items():
            if isinstance(job, dict):
                vus.add((f.name, jid, None))
                for e in job.get("steps") or []:
                    vus.add((f.name, jid, e.get("name") or e.get("uses") or "?"))
    for cle in TOLERES:
        if cle not in vus:
            fautes.append(f"tolerance perimee (emplacement disparu) : {cle}")
    return fautes


TEMOINS = [
    ("tee sans pipefail (run 42842860256)",
     "jobs:\n  j:\n    runs-on: x\n    steps:\n      - name: Bench\n        run: |\n"
     "          python3 tools/ci/bench_ladybird.py a b | tee -a \"$GITHUB_STEP_SUMMARY\"\n"),
    ("tee sans pipefail, ligne continuee",
     "jobs:\n  j:\n    runs-on: x\n    steps:\n      - name: Build\n        run: |\n"
     "          tools/ladybird/native-browser-final.sh --cible 2>&1 \\\n            | tee /tmp/b.log\n"),
    ("|| true derriere un verificateur",
     "jobs:\n  j:\n    runs-on: x\n    steps:\n      - name: V\n        shell: bash\n        run: |\n"
     "          python3 tools/verifie-ci-paliers.py || true\n"),
    ("continue-on-error non recense (job)",
     "jobs:\n  j:\n    runs-on: x\n    continue-on-error: true\n    steps:\n      - run: make\n"),
    ("continue-on-error non recense (etape)",
     "jobs:\n  j:\n    runs-on: x\n    steps:\n      - name: Verdict\n        continue-on-error: true\n"
     "        run: python3 tools/ci/verdict_convergence.py\n"),
]
POSITIFS = [
    ("tee sous pipefail", "jobs:\n  j:\n    runs-on: x\n    steps:\n      - name: B\n        run: |\n"
     "          set -o pipefail\n          python3 tools/ci/bench_ladybird.py a b | tee s\n"),
    ("tee sous shell bash", "jobs:\n  j:\n    runs-on: x\n    steps:\n      - name: B\n        shell: bash\n"
     "        run: python3 tools/ci/bench_ladybird.py a b | tee s\n"),
    ("tee d'un echo", "jobs:\n  j:\n    runs-on: x\n    steps:\n      - name: B\n        run: |\n"
     "          for f in *.bin; do echo \"$f\"; done | tee images.txt\n"),
    ("|| true sur une statistique", "jobs:\n  j:\n    runs-on: x\n    steps:\n      - name: S\n"
     "        run: ccache -s || true\n"),
]


def main() -> int:
    fautes = verifie(RACINE)
    for nom, texte in TEMOINS:
        if not verifie_workflow("temoin.yml", texte):
            fautes.append(f"temoin negatif non detecte : {nom}")
    for nom, texte in POSITIFS:
        if verifie_workflow("temoin.yml", texte):
            fautes.append(f"temoin positif refuse : {nom} -> {verifie_workflow('temoin.yml', texte)}")
    for f in fautes:
        print(f"VERDICTS_NEGATIFS_ECHEC {f}")
    if fautes:
        return 1
    print(f"VERDICTS_NEGATIFS_OK temoins={len(TEMOINS)} positifs={len(POSITIFS)} toleres={len(TOLERES)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
