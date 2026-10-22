#!/usr/bin/env python3
"""BOUCHAUD_PIPEFAIL_SANS_SIGPIPE_V1 : deux classes de verdicts faux dans les scripts.

1. FAUX ROUGE. Sous `set -o pipefail`, un tube qui finit par un lecteur qui
   s'arrete tot -- `grep -q`, `grep -m`, `head` -- fait mourir de SIGPIPE le
   producteur qui ecrit encore, et le tube echoue alors que le motif est la.
   Run 39503091649 (Integration, c5fce1fd) : os-primitives rouge sur une ligne
   PRESENTE (`sed ... | grep -q`, « couldn't write ... Broken pipe »).
   Exception : un producteur `echo` / `printf` ecrit tout d'un seul `write`.

2. FAUX VERT. `! commande` seule sur sa ligne ne declenche JAMAIS `set -e`
   (bash exclut les pipelines inverses). `! readelf -l X | grep -q INTERP`
   et `! grep -qi panic health.log` ne faisaient echouer aucun banc.

Corrige : lecteur complet (`grep ... >/dev/null`, `awk 'NR<=N'`), lecture
d'un fichier sans tube, `if ...; then exit 1; fi`. Ce garde parcourt
tools/**/*.sh et les blocs `run:` des workflows ; il rejoue aussi ses
negatifs (des extraits qui DOIVENT etre refuses).
"""
import re
import sys
from pathlib import Path

LECTEUR_TRONQUANT = re.compile(
    r"(?<!\|)\|(?!\|)\s*(?:grep\s+(?:-[A-Za-z]*q[A-Za-z]*|--quiet|-[A-Za-z]*m\s*\d+|[^|;]*\s-q\b)|head\b(?!\s+-c))")
NEGATION_SEULE = re.compile(r"^\s*!\s+\S")


def producteur(ligne, debut_lecteur):
    avant = ligne[:debut_lecteur]
    segment = re.split(r"\|\||&&|;|\b(?:el)?if\b|\bthen\b|\bwhile\b|\buntil\b|\$\(", avant)[-1]
    premier = segment.split("|")[0].strip().lstrip("!").strip()
    return premier.split()[0] if premier else ""


def fautes_texte(nom, texte, pipefail_force=None, errexit_defaut=False):
    fautes = []
    pipefail = ("pipefail" in texte) if pipefail_force is None else pipefail_force
    # Sans errexit, un tube en echec n'arrete rien -- sauf s'il est une CONDITION.
    errexit = errexit_defaut or bool(re.search(r"^\s*set\s+-[a-z]*e[a-z]*\b|^\s*set\s+-o\s+errexit", texte, re.M))
    lignes = texte.split("\n")
    for i, ligne in enumerate(lignes, 1):
        brut = ligne.strip()
        if not brut or brut.startswith("#"):
            continue
        if NEGATION_SEULE.match(ligne):
            suite = brut.endswith(("then", "do", "||", "&&", "\\")) or "||" in brut or "&&" in brut
            if not suite:
                fautes.append(f"{nom}:{i}: `! ...` seul ne declenche pas set -e (verification inerte) : {brut[:100]}")
        if not pipefail or "bash -c" in ligne or "sh -c" in ligne:
            continue
        m = LECTEUR_TRONQUANT.search(ligne)
        if not m or re.search(r"\|\|\s*(true|:)\b", ligne[m.end():]):
            continue
        if producteur(ligne, m.start()) in ("echo", "printf"):
            continue
        condition = re.search(r"\b(?:el)?if\b|\bwhile\b|\buntil\b|&&|\|\||^\s*!|\$\(", ligne)
        if not errexit and not condition:
            continue
        fautes.append(f"{nom}:{i}: lecteur tronquant sous pipefail (SIGPIPE du producteur) : {brut[:100]}")
    return fautes


def blocs_run(texte):
    """Les blocs `run: |` d'un workflow, avec leur indentation retiree."""
    lignes = texte.split("\n")
    i = 0
    while i < len(lignes):
        m = re.match(r"^(\s*)(?:- )?run:\s*\|\s*$", lignes[i])
        if not m:
            i += 1
            continue
        base = len(m.group(1))
        bloc, debut = [], i + 2
        i += 1
        while i < len(lignes) and (not lignes[i].strip() or len(lignes[i]) - len(lignes[i].lstrip()) > base):
            bloc.append(lignes[i])
            i += 1
        yield debut, "\n".join(bloc)


NEGATIFS = [
    ("sed -E 's/x//' \"$LOG\" | grep -aqE 'PROCESS_FAULT_DETAIL'", True),
    ("grep -n -A 40 '^FAILED:' /tmp/b.log | head -250", True),
    ("x=$(readelf -d \"$f\" | awk '/R/ {print $3}' | head -1)", True),
    ("  ! readelf -l \"$OUT/$f\" | grep -q INTERP", True),
    ("! grep -qi \"panic\" health.log", True),
    ("sed 's/^/  /' cc.log | grep -m1 erreur", True),
]
POSITIFS = [
    "echo \"$LIGNE\" | grep -q 'ok=1'",
    "grep -aqE 'KERNEL PANIC' \"$LOG\"",
    "sed 's/x//' \"$LOG\" | grep -aE 'X' >/dev/null",
    "grep -A 40 X b.log | awk 'NR<=250'",
    "if ! readelf -l X | grep INTERP >/dev/null; then",
    "! grep -aq X f || diag_ok=1",
    "grep -aF X \"$LOG\" | head -n 3 >&2 || true",
    "bash -c \"sed s/x// $L | grep -aq X\"",
]
POSITIFS_SANS_ERREXIT = ["sed 's/^/  /' out | head -30"]


def main():
    fautes = []
    for extrait, attendu in NEGATIFS:
        vu = bool(fautes_texte("negatif", "set -euo pipefail\n" + extrait))
        if vu != attendu:
            fautes.append(f"negatif non detecte : {extrait}")
    for extrait in POSITIFS:
        trouve = fautes_texte("positif", "set -euo pipefail\n" + extrait)
        if trouve:
            fautes.append(f"faux positif du garde : {extrait} -> {trouve[0]}")
    for extrait in POSITIFS_SANS_ERREXIT:
        trouve = fautes_texte("positif", "set -uo pipefail\n" + extrait)
        if trouve:
            fautes.append(f"faux positif du garde : {extrait} -> {trouve[0]}")
    if not fautes_texte("negatif", "set -uo pipefail\nif sed s/x// L | grep -q X; then :; fi"):
        fautes.append("negatif non detecte : condition sous pipefail sans errexit")
    n = 0
    for p in sorted(Path("tools").rglob("*.sh")):
        n += 1
        fautes += fautes_texte(str(p), p.read_text(errors="replace"))
    for p in sorted(Path(".github/workflows").glob("*.yml")):
        for debut, bloc in blocs_run(p.read_text(errors="replace")):
            n += 1
            for f in fautes_texte(f"{p}:bloc@{debut}", bloc, errexit_defaut="set +e" not in bloc):
                fautes.append(f)
    if fautes:
        print("pipefail / set -e : regle violee")
        for f in fautes:
            print("  " + f)
        return 1
    print(f"pipefail / set -e : {n} script(s) et bloc(s) sans lecteur tronquant ni negation inerte "
          f"(negatifs={len(NEGATIFS)}, positifs={len(POSITIFS)})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
