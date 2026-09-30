#!/usr/bin/env python3
"""Tout script cite par la chaine Ladybird existe DANS L'INDEX Git.

# Ce que ce garde-fou protege

Le commit p18 (b16a6fd6) a ajoute a `tools/ladybird/browser-upstream.sh`
trois appels :

    python3 tools/ladybird/prepare-p17-ipc-recovery.py "$SRC"
    python3 tools/ladybird/prepare-p15-forensics.py "$SRC"
    python3 tools/ladybird/prepare-p18-network-selfheal.py "$SRC"

sans les fichiers, qui ne sont sur aucune branche. `ladybird-native-browser`
a echoue a chaque run depuis, sur

    python3: can't open file '.../prepare-p17-ipc-recovery.py'

-- APRES la restauration de vcpkg et la preparation de quinze autres
patchers, soit plusieurs minutes de runner pour une faute visible dans
l'arbre en une seconde. Et le rouge ressemblait a une panne de la chaine de
construction, pas a un fichier oublie.

# La regle

Tout chemin `tools/**.py` ou `tools/**.sh` cite par un script de
`tools/ladybird/`, par `tools/ci/run_ladybird_browser_host.sh` ou par un
workflow `ladybird-*.yml` est un fichier suivi par Git. Le disque ne suffit
pas : un fichier present localement mais jamais ajoute est exactement le cas
d'origine.
"""

import re
import subprocess
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
CITATION = re.compile(r"\btools/[A-Za-z0-9_./-]+\.(?:py|sh)\b")


def sources() -> list[Path]:
    fichiers = sorted((RACINE / "tools" / "ladybird").glob("*.sh"))
    fichiers.append(RACINE / "tools" / "ci" / "run_ladybird_browser_host.sh")
    fichiers.extend(sorted((RACINE / ".github" / "workflows").glob("ladybird-*.yml")))
    return [f for f in fichiers if f.is_file()]


def suivis() -> set[str]:
    sortie = subprocess.run(
        ["git", "ls-files", "tools"], cwd=RACINE, capture_output=True, text=True, check=True
    ).stdout
    return set(sortie.split())


def main() -> int:
    connus = suivis()
    fautes = []
    cites = 0
    for source in sources():
        for numero, ligne in enumerate(source.read_text(encoding="utf-8").splitlines(), 1):
            if ligne.lstrip().startswith("#"):
                continue
            for chemin in CITATION.findall(ligne):
                cites += 1
                if chemin not in connus:
                    fautes.append(
                        f"  {source.relative_to(RACINE)}:{numero} cite {chemin}, "
                        f"absent de l'index Git"
                    )
    if cites == 0:
        # Zero citation trouvee : le motif ne lit plus rien, la garde serait
        # verte pour de mauvaises raisons.
        print("ECHEC : aucune citation de script trouvee -- le motif est perime")
        return 1
    if fautes:
        print("scripts de la chaine Ladybird : regle violee")
        print("\n".join(fautes))
        return 1
    print(f"ok  {cites} citation(s) de scripts dans la chaine Ladybird, toutes suivies")
    return 0


if __name__ == "__main__":
    sys.exit(main())
