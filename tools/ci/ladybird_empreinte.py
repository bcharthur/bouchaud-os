#!/usr/bin/env python3
"""Empreinte des ENTREES du build Ladybird (BOUCHAUD_LADYBIRD_EMPREINTE_V1).

Pourquoi : l'artefact Ladybird (1,1 Gio de binaires static-PIE, ~434 Mo
compresses) ne depend PAS du noyau. Il depend de la recette de build : les
scripts `tools/ladybird/**`, le SHA upstream epingle dans
`third_party/UPSTREAM.md`, et les etapes PRODUCTRICES du job `build` du
workflow. Or chaque push noyau relancait 2 h 15 de build (36 min de
dependances vcpkg + 99 min de compilation, ccache a 0 % : run 40528051026),
parce que le manifeste lie l'artefact au HEAD Bouchaud, pas a ses entrees.

L'empreinte est calculee DEPUIS GIT pour un commit donne, jamais depuis
l'arbre de travail : on peut donc la recalculer pour le HEAD qui a produit un
artefact (son manifeste le nomme) et la comparer a celle du HEAD courant.
Egalite => meme recette, meme source, meme artefact attendu.

Sous-commandes :
  calcule [--commit C]          imprime l'empreinte (defaut : HEAD)
  lie --manifest M              exige empreinte(producteur de M) == empreinte(HEAD)
  garde [--commit C]            controle statique : la recette ne lit rien
                                hors de l'empreinte, chaque etape est classee

Tout echec est rouge : fichier absent, commit introuvable, etape non classee.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path

VERSION = "BOUCHAUD_LADYBIRD_EMPREINTE_V1"
WORKFLOW = ".github/workflows/ladybird-native-browser.yml"
JOB = "build"

# Arbres et fichiers lus par la recette. `tools/ladybird/**` en entier, y
# compris ce que le build n'appelle pas : une entree de trop coute une
# reconstruction, une entree oubliee coute un faux artefact.
ARBRES = ("tools/ladybird",)
FICHIERS = (
    "third_party/UPSTREAM.md",
    "tools/ci/apt-installe.sh",
    "tools/verifie-chain-ladybird.py",
)

# Etapes du job `build` qui FABRIQUENT l'artefact (ou le corpus WPT publie a
# cote). Leur texte entre dans l'empreinte, hors cles d'orchestration.
PRODUCTRICES = (
    "Toolchain C/C++",
    "Dependances navigateur",
    "Construire Ladybird",
    "Marquer la capacite UI V16",
    "Marquer la capacite UI V19",
    "Manifeste reproductible de l'artefact",
    "Corpus WPT",
)
# Etapes `run` qui ne font que lire, mesurer ou diagnostiquer. Une etape `run`
# absente des deux listes rend la garde rouge : elle doit etre classee.
LECTRICES = (
    "Regressions des analyseurs de preuves runtime",
    "Verifier la reutilisation des sources",
    "Statistiques ccache avant",
    "Diagnostics de compilation",
    "Statistiques ccache apres",
    "Verifier les artefacts runtime",
)
# Cles d'orchestration : elles decident QUAND une etape tourne, pas ce
# qu'elle produit. Les modifier (delais, conditions) ne change pas l'artefact.
ORCHESTRATION = re.compile(r"^        (if|timeout-minutes|continue-on-error|id):")


class EmpreinteErreur(RuntimeError):
    pass


def git(*args: str, entree: bytes | None = None) -> bytes:
    try:
        return subprocess.run(["git", *args], check=True, capture_output=True, input=entree).stdout
    except (OSError, subprocess.CalledProcessError) as exc:
        detail = getattr(exc, "stderr", b"") or b""
        raise EmpreinteErreur(f"git {' '.join(args)} : {detail.decode(errors='replace').strip() or exc}") from exc


def resout(commit: str) -> str:
    sha = git("rev-parse", "--verify", f"{commit}^{{commit}}").decode().strip()
    if not re.fullmatch(r"[0-9a-f]{40}", sha):
        raise EmpreinteErreur(f"commit illisible : {commit}")
    return sha


def assure_commit(commit: str) -> None:
    """Rend `commit` lisible localement (checkout CI peu profond)."""
    try:
        resout(commit)
        return
    except EmpreinteErreur:
        pass
    git("fetch", "--quiet", "--no-tags", "--depth=1", "origin", commit)
    resout(commit)


def lit(commit: str, chemin: str) -> bytes:
    return git("show", f"{commit}:{chemin}")


def blobs(commit: str) -> list[tuple[str, str, str]]:
    """[(chemin, mode, sha256)] des entrees, triees, depuis l'arbre du commit."""
    sortie = git("ls-tree", "-r", "-z", "--full-tree", commit, "--", *ARBRES, *FICHIERS)
    entrees = []
    for brut in sortie.split(b"\0"):
        if not brut:
            continue
        meta, chemin = brut.split(b"\t", 1)
        mode, genre, objet = meta.decode().split()
        if genre != "blob":
            raise EmpreinteErreur(f"{chemin.decode()} : type {genre} non supporte")
        entrees.append((chemin.decode("utf-8", "surrogateescape"), mode, objet))
    presents = {c for c, _, _ in entrees}
    for f in FICHIERS:
        if f not in presents:
            raise EmpreinteErreur(f"entree absente au commit {commit[:12]} : {f}")
    for a in ARBRES:
        if not any(c.startswith(a + "/") for c in presents):
            raise EmpreinteErreur(f"arbre vide au commit {commit[:12]} : {a}")
    # Contenu hache en SHA-256 (l'identifiant git est du SHA-1).
    lot = git("cat-file", "--batch", entree="".join(f"{o}\n" for _, _, o in entrees).encode())
    resultat, pos = [], 0
    for chemin, mode, objet in entrees:
        fin = lot.index(b"\n", pos)
        entete = lot[pos:fin].decode().split()
        taille = int(entete[2])
        contenu = lot[fin + 1:fin + 1 + taille]
        pos = fin + 1 + taille + 1
        resultat.append((chemin, mode, hashlib.sha256(contenu).hexdigest()))
    return sorted(resultat)


def jobs(texte: str) -> dict[str, list[str]]:
    """Blocs de jobs du workflow, meme decoupage que check_pipeline_policy.py."""
    blocs: dict[str, list[str]] = {}
    courant, dans_jobs = None, False
    for ligne in texte.split("\n"):
        if ligne == "jobs:":
            dans_jobs = True
            continue
        if not dans_jobs:
            continue
        if ligne and not ligne.startswith(" ") and not ligne.startswith("#"):
            break
        if ligne.startswith("  ") and not ligne.startswith("   ") and ligne.rstrip().endswith(":") \
                and not ligne.strip().startswith("#"):
            courant = ligne.strip()[:-1]
            blocs[courant] = []
        elif courant:
            blocs[courant].append(ligne)
    return blocs


def etapes(bloc: list[str]) -> tuple[list[str], list[dict]]:
    """(entete du job hors steps, [{nom, lignes, run}]) ; commentaires retires."""
    entete, liste, dans_steps, courante = [], [], False, None
    for ligne in bloc:
        if ligne.strip().startswith("#") or not ligne.strip():
            continue
        if ligne.startswith("    steps:"):
            dans_steps = True
            continue
        if not dans_steps:
            entete.append(ligne)
            continue
        if ligne.startswith("      - "):
            courante = {"nom": None, "lignes": [], "run": False}
            liste.append(courante)
            ligne = "        " + ligne[8:]
        if courante is None:
            raise EmpreinteErreur(f"ligne hors etape : {ligne!r}")
        m = re.match(r"^        name: (.*)$", ligne)
        if m:
            courante["nom"] = m.group(1).strip().strip("'\"")
        if ligne.startswith("        run:"):
            courante["run"] = True
        courante["lignes"].append(ligne)
    return entete, liste


def recette(texte_workflow: str) -> tuple[list[str], list[dict]]:
    blocs = jobs(texte_workflow)
    if JOB not in blocs:
        raise EmpreinteErreur(f"{WORKFLOW} : job `{JOB}` absent")
    entete, liste = etapes(blocs[JOB])
    # L'entete du job compte, sauf ce qui n'est qu'orchestration.
    entete = [l for l in entete if not re.match(r"^    (needs|if|timeout-minutes|name|permissions):", l)
              and not re.match(r"^      (contents|actions): ", l)]
    return entete, liste


def classe(liste: list[dict]) -> list[str]:
    """Erreurs de classement des etapes `run` du job build."""
    erreurs = []
    vues = set()
    for e in liste:
        if not e["run"]:
            continue
        nom = e["nom"]
        if nom is None:
            erreurs.append("etape `run` sans nom : impossible a classer")
            continue
        vues.add(nom)
        if nom not in PRODUCTRICES and nom not in LECTRICES:
            erreurs.append(f"etape `run` non classee (productrice ou lectrice ?) : {nom}")
    for nom in PRODUCTRICES:
        if nom not in vues:
            erreurs.append(f"etape productrice absente du job build : {nom}")
    return erreurs


def empreinte(commit: str = "HEAD") -> dict:
    sha = resout(commit)
    h = hashlib.sha256((VERSION + "\n").encode())
    fichiers = blobs(sha)
    for chemin, mode, digest in fichiers:
        h.update(f"F\0{chemin}\0{mode}\0{digest}\n".encode("utf-8", "surrogateescape"))
    entete, liste = recette(lit(sha, WORKFLOW).decode("utf-8"))
    erreurs = classe(liste)
    if erreurs:
        raise EmpreinteErreur("; ".join(erreurs))
    h.update(("J\0" + "\n".join(entete) + "\n").encode())
    for e in liste:
        if e["nom"] in PRODUCTRICES:
            utiles = [l for l in e["lignes"] if not ORCHESTRATION.match(l)]
            h.update(("E\0" + e["nom"] + "\0" + hashlib.sha256("\n".join(utiles).encode()).hexdigest() + "\n").encode())
    return {"commit": sha, "empreinte": h.hexdigest(), "fichiers": len(fichiers)}


# --- garde statique ---------------------------------------------------------

REFERENCE = re.compile(r"(?<![A-Za-z0-9_./-])(?:\./)?(tools/[A-Za-z0-9_./-]+\.(?:sh|py|ps1|cmake|json))")
RACINE_HORS = re.compile(r"\$\{?ROOT\}?/(src|userland|kernel|Cargo\.(?:toml|lock)|rust-toolchain)")
# Verificateurs : les chemins qu'ils citent sont des DONNEES (listes de scripts
# attendus ou interdits), pas des lectures de la recette.
FEUILLES = ("tools/verifie-chain-ladybird.py",)


def couvert(chemin: str) -> bool:
    return chemin in FICHIERS or any(chemin.startswith(a + "/") for a in ARBRES)


def garde(commit: str = "HEAD") -> list[str]:
    sha = resout(commit)
    erreurs = []
    entete, liste = recette(lit(sha, WORKFLOW).decode("utf-8"))
    erreurs += classe(liste)
    # Fermeture transitive des scripts atteints par les etapes productrices.
    a_voir = []
    for e in liste:
        if e["nom"] in PRODUCTRICES:
            a_voir += REFERENCE.findall("\n".join(e["lignes"]))
    vus = set()
    while a_voir:
        chemin = a_voir.pop()
        if chemin in vus:
            continue
        vus.add(chemin)
        if not couvert(chemin):
            erreurs.append(f"la recette lit {chemin}, hors empreinte")
            continue
        if chemin in FEUILLES:
            continue
        try:
            texte = lit(sha, chemin).decode("utf-8", "replace")
        except EmpreinteErreur:
            # Couvert mais absent : l'empreinte le couvre deja (son ajout la
            # changera) ; le build, lui, echouera de lui-meme.
            continue
        for ligne in texte.split("\n"):
            code = ligne.split("#", 1)[0] if chemin.endswith(".sh") else ligne
            if chemin.endswith(".sh") and RACINE_HORS.search(code):
                erreurs.append(f"{chemin} lit {RACINE_HORS.search(code).group(0)}, hors empreinte")
            a_voir += REFERENCE.findall(code)
    if not vus:
        erreurs.append("aucun script atteint depuis les etapes productrices")
    return erreurs


def producteur(manifeste: Path) -> str:
    try:
        donnees = json.loads(manifeste.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise EmpreinteErreur(f"manifeste illisible : {exc}") from exc
    head = (donnees.get("producer") or {}).get("bouchaud_head")
    if not isinstance(head, str) or not re.fullmatch(r"[0-9a-f]{40}", head):
        raise EmpreinteErreur("manifeste sans producer.bouchaud_head valide")
    return head


def lie(manifeste: Path) -> tuple[dict, dict]:
    head = producteur(manifeste)
    assure_commit(head)
    source, courant = empreinte(head), empreinte("HEAD")
    if source["empreinte"] != courant["empreinte"]:
        raise EmpreinteErreur(
            f"entrees divergentes : producteur {head[:12]}={source['empreinte'][:16]} "
            f"courant {courant['commit'][:12]}={courant['empreinte'][:16]}")
    return source, courant


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    sub.add_parser("calcule").add_argument("--commit", default="HEAD")
    sub.add_parser("garde").add_argument("--commit", default="HEAD")
    sub.add_parser("lie").add_argument("--manifest", required=True, type=Path)
    args = p.parse_args(argv)
    try:
        if args.cmd == "calcule":
            r = empreinte(args.commit)
            print(f"LADYBIRD_EMPREINTE={r['empreinte']} commit={r['commit']} fichiers={r['fichiers']}")
        elif args.cmd == "garde":
            erreurs = garde(args.commit)
            for e in erreurs:
                print(f"LADYBIRD_EMPREINTE_GARDE_ECHEC {e}", file=sys.stderr)
            if erreurs:
                return 1
            print("LADYBIRD_EMPREINTE_GARDE_OK")
        else:
            source, courant = lie(args.manifest)
            print(f"LADYBIRD_EMPREINTE_LIEE empreinte={courant['empreinte']} "
                  f"producteur={source['commit']} courant={courant['commit']}")
    except EmpreinteErreur as exc:
        print(f"LADYBIRD_EMPREINTE_ECHEC {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
