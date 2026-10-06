#!/usr/bin/env python3
"""Installe le frontend natif Bouchaud, UI/Bouchaud, dans le worktree jetable.

BOUCHAUD_UI_V1

Ce que le navigateur devient :

    Bouchaud WM
      -> BouchaudBrowserHost (UI/Bouchaud : WebView::Application + chrome)
           |  fenetre, barre d'adresse, onglets, entree, presse-papiers,
           |  telechargements, historique, favoris
           |
           +-> WebContent    (contenu seulement, IPC upstream)
           +-> Compositor    (peint ; presente au navigateur, avec le degat)
           +-> RequestServer, ImageDecoder, WebWorker

Sources : `tools/ladybird/ui-bouchaud/` (frontend) et `tools/ladybird/chrome/`
(le chrome, en-tetes seulement). Ils sont COPIES -- jamais patches -- dans
`UI/Bouchaud/`, puis ajoutes a la construction derriere BOUCHAUD_PORT.

Le script refuse de continuer si un fichier du chrome, ou un point d'entree de
l'ancien pont M11, se trouve encore dans `Services/WebContent` : deux
architectures concurrentes ne doivent pas pouvoir coexister dans un build.
"""
from pathlib import Path
import shutil
import sys

if len(sys.argv) != 2:
    raise SystemExit("usage: prepare-ui-bouchaud.py <ladybird-worktree>")

root = Path(sys.argv[1]).resolve()
ici = Path(__file__).resolve().parent
frontend = ici / "ui-bouchaud"
chrome = ici / "chrome"
cible = root / "UI/Bouchaud"

# 1. Aucun reste du chrome dans WebContent.
webcontent = root / "Services/WebContent"
restes = sorted(p.name for p in webcontent.glob("Bouchaud*.h"))
if restes:
    raise SystemExit(f"UI/Bouchaud : chrome encore present dans WebContent : {restes}")
INTERDITS_WEBCONTENT = (
    "BouchaudChrome",
    "bouchaud_m11_start",
    "bouchaud_m9_start",
    "bouchaud_m8_start",
    "BO_GUI_FD",
    "BO_SURFACE_FD",
    "bouchaud_inject_",
    "BOUCHAUD_WEBCONTENT_FD",
)
for fichier in sorted(webcontent.glob("*.[ch]*")):
    texte = fichier.read_text(encoding="utf-8", errors="replace")
    for interdit in INTERDITS_WEBCONTENT:
        if interdit in texte:
            raise SystemExit(f"UI/Bouchaud : {interdit} dans {fichier.relative_to(root)}")

# 2. Copie du frontend et du chrome.
if cible.exists():
    shutil.rmtree(cible)
cible.mkdir(parents=True)
copies = []
for source in sorted(frontend.iterdir()):
    if source.suffix in (".h", ".cpp") or source.name == "CMakeLists.txt":
        shutil.copyfile(source, cible / source.name)
        copies.append(source.name)
for source in sorted(chrome.glob("Bouchaud*.h")):
    shutil.copyfile(source, cible / source.name)
    copies.append(source.name)
for attendu in ("CMakeLists.txt", "main.cpp", "Application.cpp", "BouchaudWebView.cpp",
                "BrowserWindow.cpp", "BouchaudChrome.h"):
    if attendu not in copies:
        raise SystemExit(f"UI/Bouchaud : {attendu} absent des sources")

# 3. Construction : a cote des services, comme l'ancien hote.
services_cmake = root / "Services/CMakeLists.txt"
data = services_cmake.read_text()
bloc = """if (BOUCHAUD_PORT)
    # BOUCHAUD_UI_V1 : le frontend natif (processus navigateur).
    add_subdirectory(${LADYBIRD_SOURCE_DIR}/UI/Bouchaud ${CMAKE_BINARY_DIR}/UI/Bouchaud)
endif()
"""
if "BouchaudBrowserHost" in data and bloc not in data:
    raise SystemExit("UI/Bouchaud : un ancien hote Services/BouchaudBrowserHost est encore construit")
if bloc not in data:
    ancre = "add_subdirectory(WebWorker)\n"
    if data.count(ancre) != 1:
        raise SystemExit("UI/Bouchaud : ancre add_subdirectory(WebWorker) introuvable")
    services_cmake.write_text(data.replace(ancre, ancre + "\n" + bloc, 1))

print(f"UI/Bouchaud installe : {len(copies)} fichiers -> {cible.relative_to(root)}")
