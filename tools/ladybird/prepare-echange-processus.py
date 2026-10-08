#!/usr/bin/env python3
"""La page laissee derriere un changement de processus est fermee.

BOUCHAUD_ECHANGE_PROCESSUS_V1

Banc memoire-onglets (run 37746917003, 07880ce6), deux phases identiques de
dix onglets autre site ouverts puis fermes :

    contextes Compositor vivants   3 -> 14 -> 22
    surfaces vivantes              2 -> 26 -> 42
    octets des surfaces            4,6 -> 71 -> 97 Mio
    RSS du WebContent de l'ouvreur +12,8 puis +11,7 Mio par phase

Un contexte et deux surfaces (1100 x 604 x 4 = 2,66 Mo chacune) de plus par
onglet, et qui ne repartent jamais : une fuite, pas un cache.

Un onglet ouvert par `window.open` nait dans le WebContent de son ouvreur,
puis navigue vers l'autre site : l'UI lui donne un NOUVEAU WebContent
(`ViewImplementation::create_new_process_for_cross_site_navigation`). Pour
l'ancien, l'UI ne fait que `unregister_view` -- elle oublie la page, mais ne
dit rien au WebContent qui l'heberge. Si ce WebContent n'a plus d'autre vue,
`close_server_if_unused` le termine et tout part avec lui. S'il en a encore
une -- l'ouvreur, cas de tout `window.open` et de tout `target=_blank` --
la page reste : son document, ses navigables, et dans le Compositor son
contexte et ses surfaces de rendu, pour toute la vie de l'ouvreur.

Correctif : avant de l'oublier, l'UI demande a l'ancien WebContent de JETER
cette page (`bouchaud_discard_page`, nouveau message). Cote WebContent : le
document est decharge, puis le traversable detruit
(`destroy_top_level_traversable`) -- donc ses navigables, et leurs contextes
Compositor (le destructeur du handle envoie `destroy_context`).

Pourquoi pas `request_close` (premiere version, run 37773872578 : 20 pages
« fermees », contextes Compositor toujours 3 -> 13 -> 22) :
`close_top_level_traversable` passe par une operation d'historique, et
l'historique d'une page vit dans l'UI (CanonicalTraversable) -- qui vient
justement de l'oublier. L'operation n'aboutit pas ; la page reste.

Ancres strictes, fail-closed, idempotent.
"""
import sys
from pathlib import Path

MARQUEUR = "BOUCHAUD_ECHANGE_PROCESSUS_V1"


def remplace(chemin: Path, ancre: str, nouveau: str) -> None:
    texte = chemin.read_text(encoding="utf-8")
    if nouveau in texte:
        return
    if texte.count(ancre) != 1:
        raise SystemExit(f"echange de processus : ancre introuvable ou ambigue dans {chemin} :\n{ancre}")
    chemin.write_text(texte.replace(ancre, nouveau, 1), encoding="utf-8")


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: prepare-echange-processus.py <arbre-ladybird>", file=sys.stderr)
        return 2
    racine = Path(sys.argv[1]).resolve()
    remplace(
        racine / "Libraries/LibWebView/ViewImplementation.cpp",
        "    if (m_client_state.client) {\n"
        "        m_client_state.client->async_notify_webdriver_of_window_replacement(m_client_state.page_index);\n"
        "        m_client_state.client->unregister_view(m_client_state.page_index);\n"
        "    }\n",
        "    if (m_client_state.client) {\n"
        "        m_client_state.client->async_notify_webdriver_of_window_replacement(m_client_state.page_index);\n"
        f"        // {MARQUEUR} : the old process may still host other views (the opener of a window.open). Close\n"
        "        // this page there, or its document, navigables and Compositor surfaces live as long as that process.\n"
        "        dbgln(\"[LB] PROCESS_SWAP_CLOSE_OLD_PAGE pid={} page={}\", m_client_state.client->pid(), m_client_state.page_index);\n"
        "        m_client_state.client->async_bouchaud_discard_page(m_client_state.page_index);\n"
        "        m_client_state.client->unregister_view(m_client_state.page_index);\n"
        "    }\n",
    )
    # Le message, et son service cote WebContent.
    remplace(
        racine / "Services/WebContent/WebContentServer.ipc",
        "    request_close(u64 page_id) =|\n",
        "    request_close(u64 page_id) =|\n"
        f"    // {MARQUEUR} : jeter la page laissee derriere un changement de processus.\n"
        "    bouchaud_discard_page(u64 page_id) =|\n",
    )
    remplace(
        racine / "Services/WebContent/ConnectionFromClient.h",
        "    virtual void request_close(u64 page_id) override;\n",
        "    virtual void request_close(u64 page_id) override;\n"
        f"    virtual void bouchaud_discard_page(u64 page_id) override; // {MARQUEUR}\n",
    )
    remplace(
        racine / "Services/WebContent/ConnectionFromClient.cpp",
        "        page->page().top_level_traversable()->close_top_level_traversable();\n}\n",
        "        page->page().top_level_traversable()->close_top_level_traversable();\n}\n"
        "\n"
        f"// {MARQUEUR}\n"
        "// The UI moved this page's view to another process and forgot it. Its session history lives in the UI, so the\n"
        "// normal close (a history operation) would never complete: unload the document, then destroy the traversable --\n"
        "// its navigables, and with them their Compositor contexts.\n"
        "void ConnectionFromClient::bouchaud_discard_page(u64 page_id)\n"
        "{\n"
        "    auto page = this->page(page_id);\n"
        "    if (!page.has_value())\n"
        "        return;\n"
        "    auto traversable = page->page().top_level_traversable();\n"
        "    if (traversable->has_been_destroyed() || traversable->is_closing())\n"
        "        return;\n"
        "    dbgln(\"[LB] PAGE_DISCARD page={}\", page_id);\n"
        "    traversable->set_closing(true);\n"
        "    auto document = traversable->active_document();\n"
        "    if (!document) {\n"
        "        traversable->destroy_top_level_traversable();\n"
        "        return;\n"
        "    }\n"
        "    document->unload_a_document_and_its_descendants({}, GC::create_function(document->heap(), [traversable] {\n"
        "        traversable->destroy_top_level_traversable();\n"
        "    }));\n"
        "}\n",
    )
    # BOUCHAUD_PAGES_MEMOIRE_V1 : distinguer racines PageHost et finalisation GC.
    # Aucun objet n'est garde vivant par ces compteurs. Le timer ne fait
    # qu'emettre un instantane de l'etat courant du PageHost.
    host_h = racine / "Services/WebContent/PageHost.h"
    host_cpp = racine / "Services/WebContent/PageHost.cpp"
    page_cpp = racine / "Services/WebContent/PageClient.cpp"

    remplace(
        host_h,
        "#include <LibGC/Root.h>\n",
        "#include <LibCore/Timer.h>\n"
        "#include <LibGC/Root.h>\n",
    )
    remplace(
        host_h,
        "namespace WebContent {\n",
        "namespace WebContent {\n\n"
        "void bouchaud_note_page_finalisee(); // BOUCHAUD_PAGES_MEMOIRE_V1\n",
    )
    remplace(
        host_h,
        "    HashMap<u64, GC::Root<PageClient>> m_pages;\n",
        "    HashMap<u64, GC::Root<PageClient>> m_pages;\n"
        "    RefPtr<Core::Timer> m_bouchaud_pages_timer;\n"
        "    u64 m_bouchaud_pages_sequence { 0 };\n",
    )

    remplace(
        host_cpp,
        "#include <WebContent/PageHost.h>\n",
        "#include <WebContent/PageHost.h>\n"
        "#include <LibCore/System.h>\n"
        "#include <stdlib.h>\n",
    )
    remplace(
        host_cpp,
        "namespace WebContent {\n",
        "namespace WebContent {\n\n"
        "// BOUCHAUD_PAGES_MEMOIRE_V1 : compteurs de diagnostic seulement.\n"
        "static u64 s_bouchaud_pages_created = 0;\n"
        "static u64 s_bouchaud_pages_detached = 0;\n"
        "static u64 s_bouchaud_pages_finalized = 0;\n"
        "void bouchaud_note_page_finalisee()\n"
        "{\n"
        "    ++s_bouchaud_pages_finalized;\n"
        "}\n",
    )
    remplace(
        host_cpp,
        "    : m_client(client)\n"
        "{\n"
        "}\n",
        "    : m_client(client)\n"
        "{\n"
        "    if (getenv(\"BOUCHAUD_LB_MEMORY_PROOF\")) {\n"
        "        m_bouchaud_pages_timer = Core::Timer::create_repeating(1000, [this] {\n"
        "            dbgln(\"[LB:PAGE_STATE] pid={} seq={} roots={} created={} detached={} finalized={} END\",\n"
        "                Core::System::getpid(),\n"
        "                ++m_bouchaud_pages_sequence,\n"
        "                m_pages.size(),\n"
        "                s_bouchaud_pages_created,\n"
        "                s_bouchaud_pages_detached,\n"
        "                s_bouchaud_pages_finalized);\n"
        "        });\n"
        "        m_bouchaud_pages_timer->start();\n"
        "    }\n"
        "}\n",
    )
    remplace(
        host_cpp,
        "    m_pages.set(page_id, PageClient::create(*this, page_id, pending_root_navigable_id));\n",
        "    m_pages.set(page_id, PageClient::create(*this, page_id, pending_root_navigable_id));\n"
        "    ++s_bouchaud_pages_created;\n",
    )
    remplace(
        host_cpp,
        "    m_pages.remove(page_id);\n",
        "    if (m_pages.remove(page_id))\n"
        "        ++s_bouchaud_pages_detached;\n",
    )
    remplace(
        host_cpp,
        "PageHost::~PageHost() = default;\n",
        "PageHost::~PageHost()\n"
        "{\n"
        "    if (m_bouchaud_pages_timer) {\n"
        "        m_bouchaud_pages_timer->on_timeout = {};\n"
        "        m_bouchaud_pages_timer->stop();\n"
        "    }\n"
        "}\n",
    )
    remplace(
        page_cpp,
        "PageClient::~PageClient() = default;\n",
        "PageClient::~PageClient()\n"
        "{\n"
        "    bouchaud_note_page_finalisee();\n"
        "}\n",
    )

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
