#!/usr/bin/env python3
"""La chaine de preparation Ladybird : ordre, contenu, et (si possible) execution.

BOUCHAUD_UI_V1

Statique (toujours) :
  1. `browser-upstream.sh` appelle les preparateurs dans l'ordre attendu, et
     `prepare-ui-bouchaud.py` en DERNIER (il verifie que WebContent est propre) ;
  2. aucun preparateur de l'ancien pont WebContent n'est appele ni present ;
  3. aucun preparateur restant n'ecrit le chrome, le canal GUI ou la route de
     capture dans `Services/WebContent` ;
  4. `prepare-network-live.route_diagnostics` ne vise plus l'ancien hote.

Dynamique (`--complet`, quand `third_party/ladybird` est present) : la chaine
entiere est executee sur un worktree jetable de l'arbre epingle, puis
`verifie-chrome.sh` controle le resultat. Quatre secondes sur un poste ; la CI
native l'execute de toute facon dans `browser-upstream.sh`.

Fail-closed ; quatre tests negatifs sur la partie statique.
"""
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
SCRIPTS = "tools/ladybird"
CHAINE = "tools/ladybird/browser-upstream.sh"

ORDRE = [
    "prepare-browser-source.py",
    "prepare-m9-source.py",
    "prepare-m9-diagnostics.py",
    "prepare-m16-dns.py",
    "prepare-dns-une-question.py",
    "prepare-fonts-systeme.py",
    "prepare-v16-fonts.py",
    "prepare-tls-diagnostic.py",
    "prepare-browser-runtime-link.py",
    "prepare-sandbox-bouchaud.py",
    "prepare-audio-bouchaud.py",
    "prepare-worker-terminate.py",
    "prepare-full-browser-host.py",
    "prepare-network-live.py",
    "prepare-ui-bouchaud.py",
]
RETIRES = [
    "prepare-m11-chrome.py", "prepare-m11-page-registry.py", "prepare-m11-input-ownership.py",
    "prepare-v19-navigateur.py", "prepare-repaint.py", "prepare-browser-host.py",
    "prepare-console.py", "prepare-image-decoder.py", "prepare-platform-complete.py",
    "prepare-m9-navigation.py",
]
# Ce qu'aucun preparateur ne doit plus ecrire (le chrome et le canal GUI
# vivent dans UI/Bouchaud ; la presentation passe par le Compositor).
INTERDITS = [
    "BouchaudChrome::", "BO_GUI_FD", "BO_SURFACE_FD", "bouchaud_m11_start",
    "bouchaud_inject_", "interactive_frame_capture", "supports_compositor() const override { return false; }",
]


def verifie(racine: Path) -> list[str]:
    fautes = []
    try:
        chaine = (racine / CHAINE).read_text(encoding="utf-8")
    except OSError as e:
        return [f"lecture impossible : {e}"]
    appels = re.findall(r"^python3 tools/ladybird/(prepare-[a-z0-9-]+\.py) \"\$SRC\"", chaine, re.M)
    if appels != ORDRE:
        fautes.append(f"{CHAINE} : ordre des preparateurs {appels} != {ORDRE}")
    for retire in RETIRES:
        if f"tools/ladybird/{retire}" in chaine and f"python3 tools/ladybird/{retire}" in chaine:
            fautes.append(f"{CHAINE} : {retire} est encore appele")
        if (racine / SCRIPTS / retire).exists():
            fautes.append(f"{SCRIPTS}/{retire} est revenu")
    for nom in ORDRE:
        chemin = racine / SCRIPTS / nom
        if not chemin.exists():
            fautes.append(f"{SCRIPTS}/{nom} absent")
            continue
        if nom == "prepare-ui-bouchaud.py":
            continue
        texte = chemin.read_text(encoding="utf-8")
        for interdit in INTERDITS:
            if interdit in texte:
                fautes.append(f"{SCRIPTS}/{nom} ecrit encore `{interdit}`")
    reseau = (racine / SCRIPTS / "prepare-network-live.py").read_text(encoding="utf-8") \
        if (racine / SCRIPTS / "prepare-network-live.py").exists() else ""
    if "Services/BouchaudBrowserHost/main.cpp" in reseau:
        fautes.append("prepare-network-live.py vise encore Services/BouchaudBrowserHost")
    return fautes


def mutation(fichier: str, a: str, b: str) -> bool:
    with tempfile.TemporaryDirectory() as tmp:
        for f in [CHAINE] + [f"{SCRIPTS}/{n}" for n in ORDRE]:
            s = RACINE / f
            if not s.exists():
                continue
            d = Path(tmp) / f
            d.parent.mkdir(parents=True, exist_ok=True)
            t = s.read_text(encoding="utf-8")
            if f == fichier:
                if a not in t:
                    return False
                t = t.replace(a, b, 1)
            d.write_text(t, encoding="utf-8")
        return bool(verifie(Path(tmp)))


def execution_complete() -> int:
    amont = RACINE / "third_party/ladybird"
    if not (amont / ".git").exists():
        print("chaine Ladybird : arbre epingle absent, execution sautee (statique seulement)")
        return 0
    with tempfile.TemporaryDirectory() as tmp:
        arbre = Path(tmp) / "lb"
        subprocess.run(["git", "-C", str(amont), "worktree", "add", "--detach", str(arbre), "HEAD"],
                       check=True, capture_output=True)
        try:
            for nom in ORDRE:
                r = subprocess.run([sys.executable, str(RACINE / SCRIPTS / nom), str(arbre)],
                                   capture_output=True, text=True)
                if r.returncode != 0:
                    print(f"chaine Ladybird : {nom} echoue\n{r.stdout[-1500:]}{r.stderr[-1500:]}")
                    return 1
            r = subprocess.run([str(RACINE / SCRIPTS / "verifie-chrome.sh"), str(arbre)],
                               capture_output=True, text=True, cwd=RACINE)
            if r.returncode != 0:
                print(f"chaine Ladybird : verifie-chrome.sh echoue\n{r.stdout[-2000:]}{r.stderr[-2000:]}")
                return 1
        finally:
            subprocess.run(["git", "-C", str(amont), "worktree", "remove", "--force", str(arbre)],
                           capture_output=True)
            shutil.rmtree(arbre, ignore_errors=True)
    print(f"chaine Ladybird : {len(ORDRE)} preparateurs executes, verifie-chrome.sh OK")
    return 0


def main() -> int:
    fautes = verifie(RACINE)
    if fautes:
        print("chaine Ladybird : regle violee")
        print("\n".join("  " + f for f in fautes))
        return 1
    negatifs = [
        (CHAINE, 'python3 tools/ladybird/prepare-ui-bouchaud.py "$SRC"\n', ""),
        (CHAINE, 'python3 tools/ladybird/prepare-network-live.py "$SRC"\n',
         'python3 tools/ladybird/prepare-network-live.py "$SRC"\npython3 tools/ladybird/prepare-m11-chrome.py "$SRC"\n'),
        (f"{SCRIPTS}/prepare-full-browser-host.py", "# 3. Le probe debugger Linux",
         "x = 'BouchaudChrome::present'\n# 3. Le probe debugger Linux"),
        (f"{SCRIPTS}/prepare-network-live.py", '    for relative in ("Services/WebContent/ConnectionFromClient.cpp",):',
         '    for relative in ("Services/BouchaudBrowserHost/main.cpp", "Services/WebContent/ConnectionFromClient.cpp",):'),
    ]
    for n, (f, a, b) in enumerate(negatifs, 1):
        if not mutation(f, a, b):
            print(f"chaine Ladybird : le test negatif {n} ne rougit pas -- garde inoperant")
            return 1
    print(f"CHAINE_LADYBIRD_OK preparateurs={len(ORDRE)} retires={len(RETIRES)} negatifs={len(negatifs)}")
    if "--complet" in sys.argv:
        return execution_complete()
    return 0


if __name__ == "__main__":
    sys.exit(main())
