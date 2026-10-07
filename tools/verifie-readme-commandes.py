#!/usr/bin/env python3
"""Toute commande citee par le README existe, avec ses parametres.

BOUCHAUD_README_COMMANDES_V1

# Ce que ce garde-fou protege

Le README est le tableau de bord du projet : on y copie une commande, on la
colle dans un terminal. Une commande inventee, un parametre renomme, un
script deplace -- et le premier geste d'un lecteur echoue. Rien ne le disait :
le README n'est execute par aucun banc.

# Les regles (blocs de code des documents listes dans DOCUMENTS)

1. Tout chemin de script cite (`.ps1`, `.py`, `.sh`, avec ou sans `./`, `.\\`)
   existe dans le depot.
2. Un `-Parametre` passe a un script PowerShell figure dans son bloc
   `param(...)` (nom ou `[Alias(...)]`).
3. Une option `--option` passee a un script Python figure dans sa source.
4. `bouchaud-lab.py <commande>` : la sous-commande existe (lue dans l'aide
   argparse du script lui-meme).
"""

import re
import subprocess
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
DOCUMENTS = ["README.md"]

SCRIPT = re.compile(r"(?:^|[\s\"'(=])((?:\.[\\/])?(?:[\w.-]+[\\/])*[\w.-]+\.(?:ps1|py|sh))\b")
BLOC = re.compile(r"^```[^\n]*\n(.*?)^```", re.S | re.M)


def params_ps1(chemin: Path) -> set[str]:
    texte = chemin.read_text(encoding="utf-8", errors="replace")
    m = re.search(r"^\s*param\s*\(", texte, re.M | re.I)
    if not m:
        return set()
    profondeur, i = 0, m.end() - 1
    while i < len(texte):
        if texte[i] == "(":
            profondeur += 1
        elif texte[i] == ")":
            profondeur -= 1
            if profondeur == 0:
                break
        i += 1
    bloc = texte[m.end():i]
    # Retirer les commentaires PowerShell : ils citent des noms.
    bloc = re.sub(r"<#.*?#>", "", bloc, flags=re.S)
    bloc = re.sub(r"#[^\n]*", "", bloc)
    noms = {n.lower() for n in re.findall(r"\$([A-Za-z_][A-Za-z0-9_]*)", bloc)}
    for alias in re.findall(r"\[Alias\(([^)]*)\)\]", bloc, re.I):
        noms |= {a.strip().strip("'\"").lower() for a in alias.split(",")}
    return noms


def sous_commandes_lab() -> set[str]:
    script = RACINE / "tools" / "remote" / "bouchaud-lab.py"
    if not script.exists():
        return set()
    sortie = subprocess.run([sys.executable, "-I", str(script), "--help"],
                            capture_output=True, text=True, timeout=60).stdout
    m = re.search(r"\{([a-z0-9,_-]+)\}", sortie)
    return set(m.group(1).split(",")) if m else set()


def resout(nom: str) -> Path | None:
    propre = nom.replace("\\", "/")
    if propre.startswith("./"):
        propre = propre[2:]
    chemin = RACINE / propre
    if chemin.exists():
        return chemin
    # Nom nu (`bouchaud-lab.py`) : accepte s'il designe un seul fichier.
    if "/" not in propre:
        trouves = [p for p in RACINE.rglob(propre)
                   if "third_party" not in p.parts and "target" not in p.parts]
        if len(trouves) == 1:
            return trouves[0]
    return None


def main() -> int:
    fautes = []
    lab = None
    # Arguments : d'autres documents a verifier (essais du garde-fou).
    for doc in sys.argv[1:] or DOCUMENTS:
        chemin_doc = RACINE / doc
        if not chemin_doc.exists():
            fautes.append(f"  {doc} absent")
            continue
        texte = chemin_doc.read_text(encoding="utf-8")
        for bloc in BLOC.finditer(texte):
            debut = texte.count("\n", 0, bloc.start()) + 2
            for k, ligne in enumerate(bloc.group(1).splitlines()):
                numero = debut + k
                code = ligne.split(" #", 1)[0] if not ligne.lstrip().startswith("#") else ""
                for m in SCRIPT.finditer(code):
                    nom = m.group(1)
                    chemin = resout(nom)
                    if chemin is None:
                        fautes.append(f"  {doc}:{numero}  script introuvable : {nom}")
                        continue
                    reste = code[m.end():]
                    # Les arguments s'arretent a un tube, un `;` ou un `&&`.
                    reste = re.split(r"\||;|&&", reste)[0]
                    if chemin.suffix == ".ps1":
                        connus = params_ps1(chemin)
                        for p in re.findall(r"(?:^|\s)-([A-Za-z][A-Za-z0-9]*)\b", reste):
                            if p.lower() not in connus:
                                fautes.append(f"  {doc}:{numero}  {nom} n'a pas de parametre -{p}")
                    elif chemin.suffix == ".py":
                        source = chemin.read_text(encoding="utf-8", errors="replace")
                        for o in re.findall(r"(?:^|\s)(--[a-z][a-z0-9-]*)", reste):
                            if f'"{o}"' not in source and f"'{o}'" not in source:
                                fautes.append(f"  {doc}:{numero}  {nom} ne declare pas {o}")
                        if chemin.name == "bouchaud-lab.py":
                            lab = sous_commandes_lab() if lab is None else lab
                            mots = [x for x in reste.split() if not x.startswith("-")]
                            if mots and mots[0] not in lab:
                                fautes.append(f"  {doc}:{numero}  bouchaud-lab.py n'a pas de commande `{mots[0]}`")
    if fautes:
        print("README : commande inexistante ou parametre invente")
        print("\n".join(fautes))
        return 1
    print("README : toutes les commandes citees existent")
    return 0


if __name__ == "__main__":
    sys.exit(main())
