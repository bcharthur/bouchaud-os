#!/usr/bin/env python3
"""Le frontend natif UI/Bouchaud : un navigateur, et WebContent ne fait que du contenu.

BOUCHAUD_UI_V1

Les regles :
  1. La vue est une `ViewImplementation` upstream (`HeadlessWebView`) et la
     trame lui arrive par `did_accept_presented_backing_store` -- le
     Compositor, jamais une capture.
  2. La fenetre lit le canal GUI dans le processus navigateur (`Core::Notifier`)
     et l'entree part par la file upstream (`enqueue_input_event`) : aucun
     evenement n'est injecte dans WebContent.
  3. Le canal GUI et la surface sont PRIVES au navigateur : FD_CLOEXEC sur les
     deux descripteurs, variables d'environnement retirees, avant le premier
     enfant.
  4. Les jalons de l'architecture sont annonces : `BOUCHAUD_UI_V1_READY`,
     `BOUCHAUD_UI_CHROME_OWNER browser`, `BOUCHAUD_UI_WEBCONTENT_CHROME 0`.
  5. Le chrome ne connait plus WebContent : namespace neutre, aucune capture
     (`ShareableBitmap`), aucun `present_complet`.
  6. Le bac a sable n'est pas desactive par defaut, et chaque service le
     verifie (`BouchaudConfinement::verifie`) au lieu d'un no-op.

Fail-closed ; huit tests negatifs.
"""
import sys
import tempfile
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
UI = "tools/ladybird/ui-bouchaud"
VUE_H, VUE = f"{UI}/BouchaudWebView.h", f"{UI}/BouchaudWebView.cpp"
FENETRE, MAIN = f"{UI}/BrowserWindow.cpp", f"{UI}/main.cpp"
CHROME = "tools/ladybird/chrome/BouchaudChrome.h"
CONFINEMENT = "tools/ladybird/sandbox/BouchaudConfinement.h"
RENDU = "tools/ladybird/sandbox/RendererSandboxBouchaud.cpp"
FICHIERS = (VUE_H, VUE, FENETRE, MAIN, CHROME, CONFINEMENT, RENDU)


def corps(texte: str, signature: str) -> str:
    i = texte.find(signature)
    if i < 0:
        return ""
    o = texte.find("{", i)
    p = 0
    for j in range(o, len(texte)):
        p += texte[j] == "{"
        p -= texte[j] == "}"
        if p == 0:
            return texte[o:j + 1]
    return ""


def code(t: str) -> str:
    return "\n".join(l.split("//", 1)[0] for l in t.splitlines())


def verifie(racine: Path) -> list[str]:
    try:
        src = {f: (racine / f).read_text(encoding="utf-8") for f in FICHIERS}
    except OSError as e:
        return [f"lecture impossible : {e}"]
    c = {f: code(t) for f, t in src.items()}
    fautes = []

    # 1. La vue upstream, la trame du Compositor.
    if "class BouchaudWebView final : public WebView::HeadlessWebView" not in c[VUE_H]:
        fautes.append(f"{VUE_H} : la vue n'est plus une HeadlessWebView upstream")
    if "virtual void did_accept_presented_backing_store(i32 bitmap_id, Gfx::IntRect damage_rect) override;" not in c[VUE_H]:
        fautes.append(f"{VUE_H} : did_accept_presented_backing_store n'est plus redefini")
    accepte = corps(c[VUE], "void BouchaudWebView::did_accept_presented_backing_store(")
    if "front_bitmap" not in accepte or "m_window.present(" not in accepte:
        fautes.append(f"{VUE} : la trame presentee (tampon de face) n'est plus remise a la fenetre")
    for f in (VUE, FENETRE):
        for interdit in ("take_screenshot", "queue_screenshot_task", "request_screenshot"):
            if interdit in c[f]:
                fautes.append(f"{f} : {interdit} -- retour a la capture")

    # 2. Canal GUI dans le navigateur, entree par la file upstream.
    demarre = corps(c[FENETRE], "void BrowserWindow::demarre(")
    if "Core::Notifier::construct(" not in demarre or "BouchaudChrome::drain()" not in demarre:
        fautes.append(f"{FENETRE} : le canal GUI n'est plus lu par le navigateur")
    for rappel in ("c.on_mouse_event = [", "c.on_key_event = ["):
        i = c[FENETRE].find(rappel)
        fin = c[FENETRE].find("\n    c.on_", i + 1)
        if i < 0 or "->enqueue_input_event(" not in c[FENETRE][i:fin]:
            fautes.append(f"{FENETRE} : {rappel.split()[0]} ne passe plus par enqueue_input_event")

    # 3. Descripteurs prives.
    init = corps(c[CHROME], "inline void initialize_from_environment()")
    if "FD_CLOEXEC" not in init or "s.gui_fd, s.surface_fd" not in init:
        fautes.append(f"{CHROME} : le canal GUI et la surface ne sont plus FD_CLOEXEC")
    for var in ('unsetenv("BO_GUI_FD")', 'unsetenv("BO_SURFACE_FD")'):
        if var not in init:
            fautes.append(f"{CHROME} : {var} absent -- un enfant saurait quels descripteurs chercher")
    if init.find("(void)enabled();") < 0 or init.find("(void)enabled();") > init.find("unsetenv("):
        fautes.append(f"{CHROME} : enabled() n'est plus fige avant le retrait de l'environnement")
    if demarre.find("initialize_from_environment()") < 0 or \
            demarre.find("initialize_from_environment()") > demarre.find("ouvre_onglet("):
        fautes.append(f"{FENETRE} : le premier WebContent naitrait avant que les descripteurs soient prives")

    # 4. Jalons.
    for jalon in ("BOUCHAUD_UI_V1_READY", "BOUCHAUD_UI_CHROME_OWNER browser", "BOUCHAUD_UI_WEBCONTENT_CHROME 0"):
        if jalon not in c[MAIN]:
            fautes.append(f"{MAIN} : jalon {jalon} absent")
    if "BOUCHAUD_UI_FIRST_FRAME" not in c[FENETRE] or "[LB:FRAME]" not in c[FENETRE]:
        fautes.append(f"{FENETRE} : la trame presentee n'est plus journalisee")

    # 5. Chrome neutre.
    if "namespace BouchaudChrome {" not in c[CHROME] or "WebContent::" in c[CHROME]:
        fautes.append(f"{CHROME} : le chrome depend de nouveau de WebContent")
    for interdit in ("ShareableBitmap last_page", "present_complet(", "wheel_handled_and_capture_requested"):
        if interdit in c[CHROME]:
            fautes.append(f"{CHROME} : {interdit} -- reste du pont de capture")

    # 6. Bac a sable.
    i = c[MAIN].find('arguments.append("--disable-sandbox");')
    if i >= 0 and "BOUCHAUD_DISABLE_SANDBOX" not in c[MAIN][max(0, i - 120):i]:
        fautes.append(f"{MAIN} : --disable-sandbox est passe par defaut")
    if "BouchaudConfinement::verifie(" not in c[RENDU]:
        fautes.append(f"{RENDU} : le rendu ne verifie plus son confinement")
    verif = corps(c[CONFINEMENT], "inline ErrorOr<void> verifie(")
    for attendu in ("PR_GET_NO_NEW_PRIVS", "socket(AF_INET", "/persist/ladybird", "/persist/Downloads",
                    "/persist/ladybird-chrome", 'ecriture_refusee("/usr"sv'):
        if attendu not in verif:
            fautes.append(f"{CONFINEMENT} : la sonde {attendu} a disparu")
    return fautes


def mutation(fichier: str, a: str, b: str) -> bool:
    with tempfile.TemporaryDirectory() as tmp:
        for f in FICHIERS:
            d = Path(tmp) / f
            d.parent.mkdir(parents=True, exist_ok=True)
            t = (RACINE / f).read_text(encoding="utf-8")
            if f == fichier:
                if a not in t:
                    return False
                t = t.replace(a, b, 1)
            d.write_text(t, encoding="utf-8")
        return bool(verifie(Path(tmp)))


def main() -> int:
    fautes = verifie(RACINE)
    if fautes:
        print("UI/Bouchaud : regle violee")
        print("\n".join("  " + f for f in fautes))
        return 1
    negatifs = [
        (VUE, "    m_window.present(*this, bitmap.release_nonnull(), largeur, hauteur, damage_rect);",
         "    vue_capture(); take_screenshot();"),
        (FENETRE, "            vue->enqueue_input_event(move(evenement));", "            (void)evenement;"),
        (CHROME, "if (drapeaux >= 0 && fcntl(fd, F_SETFD, drapeaux | FD_CLOEXEC) == 0)", "if (drapeaux >= 0)"),
        (CHROME, '    unsetenv("BO_GUI_FD");\n', ""),
        (MAIN, 'warnln("BOUCHAUD_UI_WEBCONTENT_CHROME 0");', 'warnln("ok");'),
        (CHROME, "namespace BouchaudChrome {", "namespace WebContent::BouchaudChrome {"),
        (MAIN, "    if (getenv(\"BOUCHAUD_DISABLE_SANDBOX\"))\n        arguments.append(\"--disable-sandbox\");",
         "    arguments.append(\"--disable-sandbox\");"),
        (CONFINEMENT, "int s = socket(AF_INET, SOCK_STREAM | SOCK_CLOEXEC, 0);", "int s = -1;"),
    ]
    for n, (f, a, b) in enumerate(negatifs, 1):
        if not mutation(f, a, b):
            print(f"UI/Bouchaud : le test negatif {n} ne rougit pas -- garde inoperant")
            return 1
    print(f"UI_BOUCHAUD_OK regles=6 negatifs={len(negatifs)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
