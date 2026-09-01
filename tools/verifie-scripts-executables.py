#!/usr/bin/env python3
"""Un script invoque par un workflow doit etre executable.

# Ce que ce garde-fou protege

Les neuf scripts de `tools/ci/` -- construction du noyau, boot smoke, system
health, os primitives, stress memoire SMP4, DNS ring3 -- ont ete versionnes en
mode 100644. Les workflows les invoquent directement (`run: tools/ci/x.sh`),
et chacun echouait donc sur :

    tools/ci/build_kernel.sh: Permission denied
    Process completed with exit code 126

Consequence : le job qui construit l'image echouait, et TOUTES les suites QEMU
qui en dependent etaient sautees. Le niveau integration n'a jamais rien
execute -- pas une seule fois, sur aucune PR.

C'est une faute qui ne se voit ni a la lecture du workflow, ni a la lecture du
script : les deux sont corrects. Elle ne se voit que dans le mode du fichier,
qu'on ne regarde jamais, et elle survit indefiniment parce que le seul symptome
est un job rouge que l'on prend pour un probleme du script lui-meme.

# La regle

Tout chemin `tools/**.sh` cite dans un workflow porte le bit d'execution dans
l'index Git. Le mode sur le disque ne suffit pas : c'est l'index qui est
restitue au checkout de la CI.
"""

import re
import subprocess
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
WORKFLOWS = RACINE / ".github" / "workflows"


def modes_indexes() -> dict[str, str]:
    sortie = subprocess.run(
        ["git", "ls-files", "-s"], cwd=RACINE, capture_output=True, text=True, check=True
    ).stdout
    modes = {}
    for ligne in sortie.splitlines():
        mode, _, _, chemin = ligne.replace("\t", " ").split(None, 3)
        modes[chemin] = mode
    return modes


def main() -> int:
    if not WORKFLOWS.is_dir():
        print("aucun workflow a verifier")
        return 0

    modes = modes_indexes()
    fautes = []
    verifies = 0

    for workflow in sorted(WORKFLOWS.glob("*.yml")):
        texte = workflow.read_text(encoding="utf-8")
        for trouve in sorted(set(re.findall(r"(tools/[\w/.-]+\.sh)", texte))):
            if trouve not in modes:
                fautes.append(
                    f"  {workflow.name}  invoque `{trouve}`, absent du depot"
                )
                continue
            verifies += 1
            if modes[trouve] != "100755":
                fautes.append(
                    f"  {trouve}  mode {modes[trouve]} dans l'index, invoque par "
                    f"{workflow.name} : le job echouera sur « Permission denied » "
                    f"(code 126), et tout ce qui en depend sera saute"
                )

    if fautes:
        print("scripts de workflow : regle violee")
        print("\n".join(sorted(set(fautes))))
        print()
        print("  corriger avec : git update-index --chmod=+x <script>")
        return 1

    print(f"ok  {verifies} script(s) invoque(s) par un workflow, tous executables")
    return 0


if __name__ == "__main__":
    sys.exit(main())
