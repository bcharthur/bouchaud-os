#!/usr/bin/env python3
"""
Bouchaud M11 — raccorde les pages/onglets au registre UI de WebContentClient.

Base visee : bouchaud-os a36b3e477adedeaf807992b6a75ea186a5c64271,
Ladybird epingle cdfe5f858eb5fc64a8d9d3fcc247d71b03fbd1f6.

Le bug physique bb(8) etait structurel : M11 creait page 2/3 directement dans
WebContent avec PageHost::create_page(). Le moteur connaissait donc ces pages,
mais le processus BrowserHost n'avait jamais cree le HeadlessWebView enfant ni
execute WebContentClient::register_view(). Tous les callbacks UI (backing store,
titre, URL, frame presentee...) arrivaient alors avec un page_id absent de
WebContentClient::m_views et etaient jetes.

Ce patch conserve le chemin historique sans BrowserHost, mais quand
BOUCHAUD_BROWSER_HOST est present il utilise le meme handshake que Ladybird
upstream : WebContent demande DidRequestNewWebView au BrowserHost ; le
HeadlessWebView enfant est cree et enregistre ; le host rend page_id +
root_navigable_id ; seulement ensuite WebContent cree son PageClient.
"""

from pathlib import Path
import sys

if len(sys.argv) != 2:
    raise SystemExit("usage: prepare-m11-page-registry.py <ladybird-worktree>")

root = Path(sys.argv[1])
connection_cpp = root / "Services/WebContent/ConnectionFromClient.cpp"
page_cpp = root / "Services/WebContent/PageClient.cpp"

for path in (connection_cpp, page_cpp):
    if not path.exists():
        raise SystemExit(f"M11 page registry : fichier absent: {path}")


def replace_between(data: str, start_marker: str, end_marker: str, replacement: str, label: str) -> str:
    start = data.find(start_marker)
    if start < 0:
        raise SystemExit(f"M11 page registry : debut introuvable ({label})")
    end = data.find(end_marker, start)
    if end < 0:
        raise SystemExit(f"M11 page registry : fin introuvable ({label})")
    return data[:start] + replacement + data[end:]


# ---------------------------------------------------------------------------
# 1. Onglet demande par le chrome Bouchaud (+ / Ctrl+T).
# ---------------------------------------------------------------------------
#
# Avec le BrowserHost reel, ne JAMAIS inventer le page_id dans WebContent.
# DidRequestNewWebView alloue l'identifiant cote UI puis HeadlessWebView::
# create_child() appelle initialize_client(CreateNewClient::No), qui execute
# WebContentClient::register_view(page_id, view). Le DidRequestNewWebView du
# host ne renvoie un page_id que si view_for_page_id(new_page_id) reussit : un
# retour non vide est donc deja une preuve que m_views connait la page.
#
# Sans BrowserHost on garde l'ancien chemin pour les vieux bancs M11/M9.
connection = connection_cpp.read_text()
manual_marker = "[ladybird-bouchaud] M11_TAB_HOST_REGISTERED"
if manual_marker not in connection:
    start = "    chrome.on_nouvel_onglet = [this]() -> u64 {"
    end = "\n\n    chrome.on_fermer_onglet = [this](u64 ferme) {"
    replacement = r'''    chrome.on_nouvel_onglet = [this]() -> u64 {
        if (getenv("BOUCHAUD_BROWSER_HOST") != nullptr) {
            auto response = send_sync_but_allow_failure<Messages::WebContentClient::DidRequestNewWebView>(
                page_id(), Web::HTML::ActivateTab::Yes, Web::HTML::WebViewHints {});
            if (!response) {
                warnln("[ladybird-bouchaud] M11_TAB_HOST_REQUEST_FAILED page_source={}", page_id());
                return 0;
            }
            if (!response->new_page_id().has_value() || !response->root_navigable_id().has_value()) {
                warnln("[ladybird-bouchaud] M11_TAB_HOST_REFUSED page_source={}", page_id());
                return 0;
            }

            auto const nouveau = *response->new_page_id();
            auto& client = page_host().create_page(nouveau, *response->root_navigable_id());
            // Un onglet ouvert par l'utilisateur porte immediatement un
            // document about:blank. Le root_navigable_id vient du host et est
            // donc le meme des deux cotes de l'IPC.
            Web::HTML::LocalTraversableNavigable::create_a_fresh_top_level_traversable(
                client.page(), URL::about_blank());
            client.set_maximum_frames_per_second(60.0);

            // DidRequestNewWebView ne rend `nouveau` qu'apres que le
            // HeadlessWebView enfant a ete enregistre dans m_views.
            outln("[ladybird-bouchaud] M11_TAB_HOST_REGISTERED page={} source={}", nouveau, page_id());
            outln("[ladybird-bouchaud] M11_TAB_CREATED page={}", nouveau);
            return nouveau;
        }

        // Chemin historique des bancs sans BrowserHost : ici seulement le
        // chrome et PageHost vivent dans le meme WebContent.
        auto const nouveau = BouchaudChrome::prochaine_page();
        auto& client = page_host().create_page(nouveau, page_host().allocate_navigable_id());
        Web::HTML::LocalTraversableNavigable::create_a_fresh_top_level_traversable(
            client.page(), URL::about_blank());
        client.set_maximum_frames_per_second(60.0);
        outln("[ladybird-bouchaud] M11_TAB_CREATED page={}", nouveau);
        return nouveau;
    };'''
    connection = replace_between(connection, start, end, replacement, "onglet manuel M11")
    connection_cpp.write_text(connection)


# ---------------------------------------------------------------------------
# 2. Popup / target=_blank demande par LibWeb.
# ---------------------------------------------------------------------------
#
# prepare-browser-host.py refusait cette IPC sous le vieux M9 sans host, puis
# prepare-v19-navigateur.py la remplacait par une creation directe dans
# WebContent. C'est correct uniquement pour ce legacy sans host. Quand le vrai
# BouchaudBrowserHost est present, laisser tomber jusqu'au chemin upstream :
# le host cree/enregistre le ViewImplementation, puis WebContent cree la Page.
page = page_cpp.read_text()
func_start = page.find("PageClient::NewWebViewResult PageClient::page_did_request_new_web_view(")
func_end = page.find("\nvoid PageClient::page_did_request_activate_tab()", func_start)
if func_start < 0 or func_end < 0:
    raise SystemExit("M11 page registry : fonction page_did_request_new_web_view introuvable")
segment = page[func_start:func_end]

legacy_old = "    if (bouchaud_m9_enabled()) {\n"
legacy_new = '    if (bouchaud_m9_enabled() && getenv("BOUCHAUD_BROWSER_HOST") == nullptr) {\n'
if legacy_new not in segment:
    if legacy_old not in segment:
        raise SystemExit("M11 page registry : garde legacy popup introuvable")
    segment = segment.replace(legacy_old, legacy_new, 1)

# Une popup creee par le chemin upstream est maintenant connue du host, mais le
# chrome M11 vit encore dans WebContent. L'ajouter a sa bande seulement APRES
# la reponse host garde les deux registres synchronises.
popup_marker = "[ladybird-bouchaud] M11_TAB_POPUP_HOST_REGISTERED"
if popup_marker not in segment:
    anchor = "    auto& new_client = m_owner.create_page(*response->new_page_id(), *response->root_navigable_id());\n"
    if anchor not in segment:
        raise SystemExit("M11 page registry : creation popup upstream introuvable")
    extra = anchor + r'''#if defined(BOUCHAUD_PORT)
    if (bouchaud_m9_enabled() && getenv("BOUCHAUD_BROWSER_HOST") != nullptr && BouchaudChrome::enabled()) {
        BouchaudChrome::ajoute_onglet(*response->new_page_id(), ByteString {},
            activate_tab == Web::HTML::ActivateTab::Yes);
        outln("[ladybird-bouchaud] M11_TAB_POPUP_HOST_REGISTERED page={} active={}",
            *response->new_page_id(), activate_tab == Web::HTML::ActivateTab::Yes ? 1 : 0);
    }
#endif
'''
    segment = segment.replace(anchor, extra, 1)

page = page[:func_start] + segment + page[func_end:]
page_cpp.write_text(page)


# ---------------------------------------------------------------------------
# 3. Preuves statiques sur le worktree PREPARE, pas seulement sur ce script.
# ---------------------------------------------------------------------------
connection = connection_cpp.read_text()
page = page_cpp.read_text()

if manual_marker not in connection:
    raise SystemExit("M11 page registry : l'onglet manuel ne passe pas par le host")
if "send_sync_but_allow_failure<Messages::WebContentClient::DidRequestNewWebView>" not in connection:
    raise SystemExit("M11 page registry : IPC DidRequestNewWebView absente de l'onglet manuel")

func_start = page.find("PageClient::NewWebViewResult PageClient::page_did_request_new_web_view(")
func_end = page.find("\nvoid PageClient::page_did_request_activate_tab()", func_start)
segment = page[func_start:func_end]
if legacy_new not in segment:
    raise SystemExit("M11 page registry : le BrowserHost reste bloque par le fallback legacy")
if popup_marker not in segment:
    raise SystemExit("M11 page registry : la popup host n'est pas raccordee au chrome")

print("Bouchaud M11 page registry applique : onglets/popup passent par WebContentClient::m_views avec BrowserHost")
