#!/usr/bin/env python3
"""Chaque onglet est une vue enregistree aupres de son WebContent.

BOUCHAUD_PAGE_LIFECYCLE_V2 (BOUCHAUD_UI_V1)

Le defaut bb(8) : le chrome M11 creait les pages 2 et 3 DANS WebContent
(`PageHost::create_page`), si bien que `WebContentClient::m_views` ne les
connaissait pas et jetait tout ce qu'elles envoyaient. Depuis UI/Bouchaud, un
onglet n'existe que comme `BouchaudWebView`, c'est-a-dire une
`ViewImplementation` que `initialize_client` enregistre :

  1. un onglet ouvert par l'utilisateur : `BouchaudWebView::create` ->
     `initialize_client(CreateNewClient::Yes)` (WebContent neuf) ;
  2. une page ouverte par le document (`window.open`) : `create_child` avec le
     `page_index` que l'hote a alloue -> `initialize_client(CreateNewClient::No)` ;
  3. la vue redirige `on_new_web_view` vers la fenetre (sinon `HeadlessWebView`
     fabriquerait des vues sans onglet) ;
  4. la fenetre annonce l'onglet au chrome APRES avoir cree et branche la vue ;
  5. aucune source du frontend ne cree de page cote moteur.

Fail-closed ; cinq tests negatifs.
"""
import re
import sys
import tempfile
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
VUE = "tools/ladybird/ui-bouchaud/BouchaudWebView.cpp"
FENETRE = "tools/ladybird/ui-bouchaud/BrowserWindow.cpp"
FICHIERS = (VUE, FENETRE, "tools/ladybird/ui-bouchaud/Application.cpp", "tools/ladybird/ui-bouchaud/main.cpp")


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


def verifie(racine: Path) -> list[str]:
    try:
        src = {f: (racine / f).read_text(encoding="utf-8") for f in FICHIERS}
    except OSError as e:
        return [f"lecture impossible : {e}"]
    fautes = []
    v, w = src[VUE], src[FENETRE]
    if "initialize_client(CreateNewClient::Yes)" not in corps(v, "BouchaudWebView::create(BrowserWindow&"):
        fautes.append(f"{VUE} : create n'enregistre plus la vue sur un WebContent neuf")
    enfant = corps(v, "BouchaudWebView::create_child(")
    if "m_client_state.page_index = page_index;" not in enfant or "initialize_client(CreateNewClient::No)" not in enfant:
        fautes.append(f"{VUE} : create_child n'enregistre plus la page allouee par l'hote")
    if "on_new_web_view = [this]" not in v or "ouvre_vue_demandee_par_la_page" not in v:
        fautes.append(f"{VUE} : on_new_web_view n'est plus redirige vers la fenetre")
    for sig in ("u64 BrowserWindow::ouvre_onglet(", "String BrowserWindow::ouvre_vue_demandee_par_la_page("):
        c = corps(w, sig)
        cree = c.find("BouchaudWebView::create")
        branche = c.find("branche_vue(*vue)")
        annonce = c.find("BouchaudChrome::ajoute_onglet")
        if min(cree, branche, annonce) < 0 or not (cree < branche < annonce):
            fautes.append(f"{FENETRE} : {sig.split('::')[1]} n'annonce plus l'onglet APRES la vue")
    for f, t in src.items():
        for interdit in ("create_page(", "PageHost", "create_a_fresh_top_level_traversable"):
            if interdit in t:
                fautes.append(f"{f} : {interdit} -- le frontend cree une page cote moteur")
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
        print("cycle de vie des pages : regle violee")
        print("\n".join("  " + f for f in fautes))
        return 1
    negatifs = [
        (VUE, "    view->initialize_client(CreateNewClient::Yes);", "    (void)view;"),
        (VUE, "    view->m_client_state.page_index = page_index;\n", ""),
        (VUE, "    on_new_web_view = [this]", "    on_new_web_view_inutilise = [this]"),
        (FENETRE, "    branche_vue(*vue);\n    vue->set_system_visibility_state",
         "    BouchaudChrome::ajoute_onglet(0, {}, false);\n    branche_vue(*vue);\n    vue->set_system_visibility_state"),
        (FENETRE, "static constexpr u64 trames_journalisees",
         "static void x(WebContent::PageHost& h) { h.create_page(1, {}); }\nstatic constexpr u64 trames_journalisees"),
    ]
    for n, (f, a, b) in enumerate(negatifs, 1):
        if not mutation(f, a, b):
            print(f"cycle de vie des pages : le test negatif {n} ne rougit pas -- garde inoperant")
            return 1
    print(f"BOUCHAUD_PAGE_LIFECYCLE_GUARD_OK negatifs={len(negatifs)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
