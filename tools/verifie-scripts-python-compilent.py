#!/usr/bin/env python3
"""Garde : chaque script Python des bancs et de la chaine COMPILE.

BOUCHAUD_SCRIPTS_PYTHON_COMPILENT_V1

Run 37589065373 (3a0d4613) : la fixture des bancs navigateur
(tools/health/browser_host_fixture.py) ne demarrait plus -- « SyntaxError:
bytes can only contain ASCII literal characters », des guillemets « » dans
une chaine b\"\"\"...\"\"\". Quatre jobs rouges en deux minutes (smoke,
endurance, robustesse, ordre), et un rapport qui ne disait que « kill: No
such process ». Une faute que la compilation seule attrape, et que rien ne
verifiait avant la CI.

Compile (sans executer) tout .py sous tools/ ; echoue sur le premier qui ne
compile pas, avec son message.
"""
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]


def main() -> int:
    fautes = []
    fichiers = sorted(p for p in (RACINE / "tools").rglob("*.py") if "__pycache__" not in p.parts)
    for chemin in fichiers:
        try:
            compile(chemin.read_bytes(), str(chemin), "exec", dont_inherit=True)
        except (SyntaxError, ValueError) as erreur:
            fautes.append(f"{chemin.relative_to(RACINE)}:{getattr(erreur, 'lineno', '?')} : {erreur.__class__.__name__}: {getattr(erreur, 'msg', erreur)}")
    if fautes:
        for faute in fautes:
            print(f"ECHEC {faute}")
        return 1
    print(f"scripts Python : {len(fichiers)} compilent")
    return 0


if __name__ == "__main__":
    sys.exit(main())
