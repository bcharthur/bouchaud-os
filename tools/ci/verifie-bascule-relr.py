#!/usr/bin/env python3
"""Verifie la bascule BOUCHAUD_HELPER_RELR sur un arbre Ladybird factice.

POURQUOI CE TEST EXISTE
-----------------------
Le run 35920701144 a perdu une experience complete (~45 min de CI) parce que
`build-variante-*.sh` pointait sur `third_party/ladybird` alors que l'arbre
reellement prepare et configure par CMake est
`third_party/ladybird-browser-prepared`. L'etape a tourne 0 s, masquee par
`continue-on-error: true`, et le job `variante-elf` a echoue sur un artefact
qui n'avait jamais ete produit.

La lecon n'est pas « corriger le chemin ». Elle est : le maillon
`prepare-browser-runtime-link.py` -> options de lien effectives n'avait JAMAIS
ete teste hors d'un build Ladybird de 40 minutes. Ce fichier ferme ce trou avec
une boucle courte : un arbre factice qui porte exactement les motifs d'ancrage
amont, la bascule appliquee dans les deux sens, et des assertions fail-closed.

CE QUE CE TEST PROUVE
---------------------
1. les six cibles runtime Bouchaud recoivent bien un bloc de lien ;
2. `BOUCHAUD_HELPER_RELR=1` ajoute `-z pack-relative-relocs` au lien
   static-PIE ;
3. le defaut reste le static-PIE nu ;
4. rejouer le script n'empile pas les blocs (idempotence), dans les deux sens
   de la bascule -- c'est exactement ce que fait le `trap restaure_lien EXIT`
   de `build-variante-relr.sh`, et un retour rate produirait des aides RELR a
   l'insu de tous.

CE QUE CE TEST NE PROUVE PAS
----------------------------
Il ne prouve pas que l'ELF resultant demarre sous Bouchaud, ni que RELR change
le temps `exec -> main`. Cela, seule la mesure A/B le dira -- et elle n'a de
sens qu'apres un partage utilisateur/noyau honnete du segment (BOUCHAUD_C66).
"""
from pathlib import Path
import os
import shutil
import subprocess
import sys
import tempfile

RACINE = Path(__file__).resolve().parents[2]
SCRIPT = RACINE / "tools/ladybird/prepare-browser-runtime-link.py"

CIBLES = ["WebContent", "RequestServer", "ImageDecoder", "WebWorker", "Compositor", "WebDriver"]

ATTENDU_PIE = "-static-pie LINKER:--allow-multiple-definition"
ATTENDU_RELR = "-static-pie LINKER:-z,pack-relative-relocs LINKER:--allow-multiple-definition"

# Chaque entree porte le motif d'ancrage amont EXACT attendu par
# prepare-browser-runtime-link.py. Si l'amont bouge, ce test casse -- et c'est
# precisement le signal voulu : le script de preparation casserait aussi.
FICHIERS = {
    "Services/WebContent/CMakeLists.txt": "add_executable(WebContent main.cpp)\n",
    "Services/RequestServer/CMakeLists.txt":
        "set(SOURCES main.cpp)\nif (LINUX)\n    list(APPEND SOURCES SandboxLinux.cpp)\nendif()\n",
    "Services/ImageDecoder/CMakeLists.txt":
        "add_executable(ImageDecoder main.cpp)\nif (LINUX)\n    target_sources(ImageDecoder PRIVATE SandboxLinux.cpp)\nendif()\n",
    "Services/WebWorker/CMakeLists.txt":
        "add_executable(WebWorker main.cpp)\nif (LINUX)\n    target_sources(WebWorker PRIVATE ../RendererSandboxLinux.cpp)\nendif()\n",
    "Services/Compositor/CMakeLists.txt":
        "add_executable(Compositor main.cpp)\nif (LINUX)\n    target_sources(Compositor PRIVATE SandboxLinux.cpp)\nendif()\n",
    "Services/WebDriver/CMakeLists.txt": "add_executable(WebDriver main.cpp)\n",
    "Services/WebContent/PageClient.h":
        "class PageClient {\n    virtual bool supports_compositor() const override { return true; }\n};\n",
    "Libraries/LibWebView/Utilities.cpp":
        '''void platform_init()
{
    s_ladybird_resource_root = [] {
        auto home = Core::Environment::get("XDG_CONFIG_HOME"sv)
            .value_or("/"sv);
        return ByteString { home };
#endif
    }();

    Core::ResourceImplementation::install(make<Core::ResourceImplementationFile>(MUST(String::from_byte_string(s_ladybird_resource_root))));
}
''',
    "Libraries/LibWeb/HTML/LocalNavigable.cpp":
        '''#include <LibWeb/Painting/DisplayListRecordingContext.h>

void LocalNavigable::render_screenshot(Gfx::PaintingSurface& painting_surface, PaintConfig paint_config, Function<void()>&& callback)
{
    if (!has_compositor_context()) {
        callback();
        return;
    }

    if (!record_display_list_and_scroll_state(paint_config)) {
        callback();
        return;
    }
    compositor_context().request_screenshot(painting_surface, move(callback));
}
''',
    "Services/WebContent/ConnectionFromClient.cpp":
        '''#include <LibWeb/HTML/LocalTraversableNavigable.h>

void ConnectionFromClient::bootstrap(u64 page_id, u64 root_navigable_id, int width, int height)
{
    initialize(page_id, root_navigable_id, allocator);

    auto viewport = Gfx::IntSize { width, height }.to_type<Web::DevicePixels>();
    update_screen_rects(page_id, Vector<Web::DevicePixelRect> { screen }, 0);
    set_viewport(page_id, viewport, 1.0, Web::ViewportIsFullscreen::No);
    set_window_size(page_id, viewport);
    set_has_focus(page_id, true);
    set_system_visibility_state(page_id, Web::HTML::VisibilityState::Visible);

    static constexpr auto html = "<html></html>";
    outln("[ladybird-bouchaud] M8_BOOTSTRAP page={} viewport={}x{}", page_id, width, height);
    load_html(page_id, ByteString { html });
}
''',
}


def fabrique_arbre(base: Path) -> None:
    for relatif, contenu in FICHIERS.items():
        chemin = base / relatif
        chemin.parent.mkdir(parents=True, exist_ok=True)
        chemin.write_text(contenu)


def prepare(base: Path, relr: bool) -> None:
    env = dict(os.environ)
    env["BOUCHAUD_HELPER_RELR"] = "1" if relr else "0"
    proc = subprocess.run(
        [sys.executable, str(SCRIPT), str(base)],
        env=env, capture_output=True, text=True,
    )
    if proc.returncode != 0:
        raise SystemExit(
            "BASCULE_RELR_FAIL etape=prepare relr={} rc={}\n{}{}".format(
                int(relr), proc.returncode, proc.stdout, proc.stderr
            )
        )


def controle(base: Path, attendu: str, refuse: str, etiquette: str) -> list:
    erreurs = []
    for cible in CIBLES:
        chemin = base / f"Services/{cible}/CMakeLists.txt"
        texte = chemin.read_text()
        marqueur = f"# Bouchaud runtime link policy for {cible}"
        blocs = texte.count(marqueur)
        if blocs != 1:
            erreurs.append(f"{etiquette}: {cible} porte {blocs} bloc(s) de lien, attendu 1")
            continue
        ligne = f"target_link_options({cible} PRIVATE {attendu})"
        if ligne not in texte:
            erreurs.append(f"{etiquette}: {cible} n'a pas `{ligne}`")
        if refuse and f"PRIVATE {refuse})" in texte:
            erreurs.append(f"{etiquette}: {cible} porte encore l'option de l'autre mode")
        if "SKIP_BUILD_RPATH TRUE" not in texte:
            erreurs.append(f"{etiquette}: {cible} a perdu la purge de RPATH")
    return erreurs


def essai(nom: str, etapes: list) -> list:
    """Une bascule = une suite d'appels au script, chaque etape controlee."""
    base = Path(tempfile.mkdtemp(prefix="bascule-relr-"))
    try:
        fabrique_arbre(base)
        erreurs = []
        for relr, attendu, refuse in etapes:
            prepare(base, relr)
            erreurs += controle(base, attendu, refuse, f"{nom}/relr={int(relr)}")
        return erreurs
    finally:
        shutil.rmtree(base, ignore_errors=True)


def test_negatif() -> list:
    """Le controle doit REFUSER un arbre ou une cible n'a pas ete patchee.

    Sans ce test, `controle()` pourrait ne rien verifier du tout et ce fichier
    serait vert par construction.
    """
    base = Path(tempfile.mkdtemp(prefix="bascule-relr-neg-"))
    try:
        fabrique_arbre(base)
        prepare(base, False)
        # On defait le patch de WebWorker : le controle DOIT le voir.
        chemin = base / "Services/WebWorker/CMakeLists.txt"
        texte = chemin.read_text()
        chemin.write_text(texte[: texte.index("# Bouchaud runtime link policy for WebWorker")])
        vu = controle(base, ATTENDU_PIE, ATTENDU_RELR, "negatif")
        if not any("WebWorker" in e for e in vu):
            return ["test negatif inerte : une cible non patchee n'a pas ete detectee"]
        return []
    finally:
        shutil.rmtree(base, ignore_errors=True)


def main() -> int:
    erreurs = []
    erreurs += essai("defaut", [(False, ATTENDU_PIE, ATTENDU_RELR)])
    erreurs += essai("relr", [(True, ATTENDU_RELR, ATTENDU_PIE)])
    # Idempotence : rejouer le meme mode ne doit pas empiler de bloc.
    erreurs += essai("rejeu", [
        (False, ATTENDU_PIE, ATTENDU_RELR),
        (False, ATTENDU_PIE, ATTENDU_RELR),
    ])
    # Aller-retour : c'est exactement ce que fait le `trap restaure_lien EXIT`
    # de build-variante-relr.sh. Si le retour laissait l'arbre en RELR,
    # le build principal produirait des aides non-PIE a l'insu de tous.
    erreurs += essai("aller-retour", [
        (False, ATTENDU_PIE, ATTENDU_RELR),
        (True, ATTENDU_RELR, ATTENDU_PIE),
        (False, ATTENDU_PIE, ATTENDU_RELR),
        (True, ATTENDU_RELR, ATTENDU_PIE),
    ])
    erreurs += test_negatif()

    if erreurs:
        for e in erreurs:
            print(f"BASCULE_RELR_FAIL {e}")
        print(f"BASCULE_RELR_VERDICT echec erreurs={len(erreurs)}")
        return 1
    print(f"BASCULE_RELR_VERDICT ok cibles={len(CIBLES)} option=pack-relative-relocs")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
