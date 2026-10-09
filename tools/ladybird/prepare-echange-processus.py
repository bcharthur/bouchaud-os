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
        "    auto browsing_context = traversable->active_browsing_context();\n"
        "    dbgln(\"[LB] PAGE_DISCARD_BROWSING_CONTEXT_CAPTURED page={} present={}\", page_id, browsing_context ? 1 : 0);\n"
        "    auto document = traversable->active_document();\n"
        "    if (!document) {\n"
        "        traversable->bouchaud_destroy_top_level_traversable_after_document_destruction(browsing_context);\n"
        "        return;\n"
        "    }\n"
        "    document->unload_a_document_and_its_descendants({}, GC::create_function(document->heap(), [traversable, browsing_context] {\n"
        "        traversable->bouchaud_destroy_top_level_traversable_after_document_destruction(browsing_context);\n"
        "    }));\n"
        "}\n",
    )
    # BOUCHAUD_P13_LIFECYCLE_CLOSURE_V6 : le retrait de PageHost est la DERNIERE etape.
    # Le Document est detruit de facon asynchrone. La connexion capture le BrowsingContext
    # AVANT unload (qui peut detruire Document et annuler active_document), puis retire
    # l'UI/PageHost seulement dans le callback de fin de destruction.
    remplace(
        racine / "Libraries/LibWeb/HTML/LocalTraversableNavigable.h",
        "    void destroy_top_level_traversable();\n",
        "    void destroy_top_level_traversable();\n"
        "    void bouchaud_destroy_top_level_traversable_after_document_destruction(GC::Ptr<BrowsingContext> browsing_context); // BOUCHAUD_P13_LIFECYCLE_CLOSURE_V4\n",
    )
    remplace(
        racine / "Libraries/LibWeb/HTML/LocalTraversableNavigable.cpp",
        "// https://html.spec.whatwg.org/multipage/interaction.html#system-visibility-state\n",
        "// BOUCHAUD_P13_LIFECYCLE_CLOSURE_V4\n"
        "void LocalTraversableNavigable::bouchaud_destroy_top_level_traversable_after_document_destruction(GC::Ptr<BrowsingContext> browsing_context)\n"
        "{\n"
        "    VERIFY(is_top_level_traversable());\n"
        "    if (has_been_destroyed())\n"
        "        return;\n"
        "\n"
        "    auto finish = GC::create_function(heap(), [this, browsing_context] {\n"
        "        if (has_been_destroyed())\n"
        "            return;\n"
        "        dbgln(\"[LB] PAGE_DISCARD_DOCUMENT_DESTROY_END navigable={}\", id());\n"
        "        if (!browsing_context)\n"
        "            dbgln(\"[LB] PAGE_DISCARD_NO_BROWSING_CONTEXT navigable={}\", id());\n"
        "        else {\n"
        "            browsing_context->remove();\n"
        "            dbgln(\"[LB] PAGE_DISCARD_BROWSING_CONTEXT_REMOVED navigable={}\", id());\n"
        "        }\n"
        "        auto& discarded_page = page();\n"
        "        discarded_page.client().page_did_close_top_level_traversable();\n"
        "        user_agent_top_level_traversable_set().remove(this);\n"
        "        set_has_been_destroyed();\n"
        "        remove_from_all_local_navigables();\n"
        "        // PageHost has already removed the GC root. A retained closed WindowProxy must not retain PageClient.\n"
        "        discarded_page.bouchaud_release_client_after_discard();\n"
        "        dbgln(\"[LB] PAGE_DISCARD_CLIENT_RELEASED navigable={}\", id());\n"
        "        dbgln(\"[LB] PAGE_DISCARD_END navigable={}\", id());\n"
        "    });\n"
        "\n"
        "    if (auto document = active_document()) {\n"
        "        dbgln(\"[LB] PAGE_DISCARD_DOCUMENT_DESTROY_BEGIN navigable={}\", id());\n"
        "        document->destroy_a_document_and_its_descendants(finish);\n"
        "        return;\n"
        "    }\n"
        "    finish->function()();\n"
        "}\n"
        "\n"
        "// https://html.spec.whatwg.org/multipage/interaction.html#system-visibility-state\n",
    )
    remplace(
        racine / "Libraries/LibWeb/DOM/Document.cpp",
        "    // 2. Abort document.\n"
        "    abort();\n",
        "    // BOUCHAUD_P13_LIFECYCLE_CLOSURE_V4 : un parser-end encore actif porte\n"
        "    // un Timer activity-root. A la destruction definitive du Document, il ne\n"
        "    // doit plus pouvoir retenir HTMLDocument -> Page -> PageClient.\n"
        "    if (m_html_parser_end_state) {\n"
        "        m_html_parser_end_state->cancel();\n"
        "        m_html_parser_end_state = nullptr;\n"
        "    }\n"
        "\n"
        "    // 2. Abort document.\n"
        "    abort();\n",
    )

    # BOUCHAUD_P13_PAGECLIENT_DISCARD_V5 : une fermeture peut laisser un WindowProxy
    # atteignable par le JS de l'ouvreur (Promise/GeneratorObject). Ce proxy garde
    # le Page GC, qui ne doit plus garder un PageClient ferme une fois la fermeture
    # et la destruction des contexts Compositor entierement terminees.
    # Le Page de navigation normal garde son client tant que PageHost le possede.
    remplace(
        racine / "Libraries/LibWeb/Page/Page.h",
        "    PageClient& client() { return m_client; }\n"
        "    PageClient const& client() const { return m_client; }\n",
        "    PageClient& client() { VERIFY(m_client); return *m_client; }\n"
        "    PageClient const& client() const { VERIFY(m_client); return *m_client; }\n"
        "    void bouchaud_release_client_after_discard(); // BOUCHAUD_P13_PAGECLIENT_DISCARD_V5\n",
    )
    remplace(
        racine / "Libraries/LibWeb/Page/Page.h",
        "    GC::Ref<PageClient> m_client;\n",
        "    GC::Ptr<PageClient> m_client; // BOUCHAUD_P13_PAGECLIENT_DISCARD_V5\n",
    )
    remplace(
        racine / "Libraries/LibWeb/Page/Page.cpp",
        "Page::~Page() = default;\n",
        "Page::~Page() = default;\n"
        "\n"
        "// BOUCHAUD_P13_PAGECLIENT_DISCARD_V5 : called only after PageHost unroots a fully destroyed page.\n"
        "void Page::bouchaud_release_client_after_discard()\n"
        "{\n"
        "    VERIFY(m_client);\n"
        "    m_client = nullptr;\n"
        "}\n",
    )

    # BOUCHAUD_P13_CONSOLE_LIFECYCLE_V6 : la racine du dernier PageClient
    # passe par WindowProxy -> Window -> ancien Document -> DevToolsConsoleClient.
    # La destruction definitive du Document retire ses clients console, apres
    # unload/cleanup. La console de JS conserve son objet mais plus ce client :
    # les vieux WindowProxy restent valides (closed), sans retenir PageClient.
    remplace(
        racine / "Libraries/LibJS/Console.h",
        "    void set_client(ConsoleClient& client) { m_client = &client; }\n",
        "    void set_client(ConsoleClient& client) { m_client = &client; }\n"
        "    void bouchaud_clear_client_if(ConsoleClient const* client) // BOUCHAUD_P13_CONSOLE_LIFECYCLE_V6\n"
        "    {\n"
        "        if (m_client.ptr() == client)\n"
        "            m_client = nullptr;\n"
        "    }\n",
    )
    remplace(
        racine / "Libraries/LibWeb/DOM/Document.cpp",
        "#include <LibJS/Console.h>\n",
        "#include <LibJS/Console.h>\n"
        "#include <LibJS/Runtime/ConsoleObject.h>\n",
    )
    remplace(
        racine / "Libraries/LibWeb/DOM/Document.cpp",
        "    // AD-HOC: Destruction does not go through did_stop_being_active_document_in_navigable(),\n",
        "    // BOUCHAUD_P13_CONSOLE_LIFECYCLE_V6 : detach the obsolete console client\n"
        "    // only after abort/unloading cleanup, before the destroyed Document remains\n"
        "    // reachable through a closed WindowProxy in its opener's JS promises.\n"
        "    if (m_console_client) {\n"
        "        auto console_object = relevant_settings_object().realm().intrinsics().console_object();\n"
        "        console_object->console().bouchaud_clear_client_if(m_console_client.ptr());\n"
        "        m_console_client = nullptr;\n"
        "    }\n"
        "    // AD-HOC: Destruction does not go through did_stop_being_active_document_in_navigable(),\n",
    )

    # BOUCHAUD_P13_GC_PHYSICAL_OBSERVATION_V10 : TEMPORARY, opt-in.
    # V9 (early wake after 32 freed blocks) did not improve RSS and is
    # reverted. Count real LibGC 2-MiB chunks committed and 16-KiB blocks
    # decommitted, only when BOUCHAUD_LB_MEMORY_PROOF=1. No forced GC,
    # allocator policy or test threshold changes.
    remplace(
        racine / "Libraries/LibGC/BlockAllocator.cpp",
        "#include <AK/Assertions.h>\n",
        "#include <AK/Assertions.h>\n"
        "#include <AK/Format.h>\n"
        "#include <stdlib.h>\n",
    )
    remplace(
        racine / "Libraries/LibGC/BlockAllocator.cpp",
        "        MUST(Core::System::commit_memory(chunk, CHUNK_SIZE));\n"
        "        m_next_chunk_offset += CHUNK_SIZE;\n"
        "        return chunk;\n",
        "        MUST(Core::System::commit_memory(chunk, CHUNK_SIZE));\n"
        "        m_next_chunk_offset += CHUNK_SIZE;\n"
        "        // BOUCHAUD_P13_GC_PHYSICAL_OBSERVATION_V10 : read-only chunk counter.\n"
        "        if (getenv(\"BOUCHAUD_LB_MEMORY_PROOF\"))\n"
        "            dbgln(\"[LB:P13_GC_CHUNK] pid={} chunk={:p} count={} END\", Core::System::getpid(), chunk, m_next_chunk_offset / CHUNK_SIZE);\n"
        "        return chunk;\n",
    )
    remplace(
        racine / "Libraries/LibGC/BlockAllocator.cpp",
        "    {\n"
        "        Sync::MutexLocker locker(a.m_mutex);\n"
        "        for (auto* slot : to_process)\n"
        "            a.m_blocks.append(slot);\n"
        "    }\n"
        "}\n",
        "    {\n"
        "        Sync::MutexLocker locker(a.m_mutex);\n"
        "        for (auto* slot : to_process)\n"
        "            a.m_blocks.append(slot);\n"
        "    }\n"
        "    if (getenv(\"BOUCHAUD_LB_MEMORY_PROOF\") && !to_process.is_empty()) {\n"
        "        static size_t total_decommitted = 0;\n"
        "        total_decommitted += to_process.size();\n"
        "        dbgln(\"[LB:P13_GC_DECOMMIT] pid={} batch={} total={} END\",\n"
        "            Core::System::getpid(), to_process.size(), total_decommitted);\n"
        "    }\n"
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
