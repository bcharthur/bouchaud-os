#!/usr/bin/env python3
"""Le lien Compositor <-> WebContent peut mourir seul : le dire a l'UI, et
s'en relever.

BOUCHAUD_COMPOSITOR_LIEN_V1 (convergence P1)

Le crash (journal utilisateur, binaires anterieurs a 0ae5cf42) :

    WebContent(...): Failed to receive message_id: 41
    VERIFICATION FAILED: connection at Services/Compositor/ConnectionFromClient.cpp:68
    Compositor ... code=139

Le message 41 du point d'entree CompositorWebContentServer est
`TakePendingAsyncScrollUpdatesResponse` : WebContent attendait la reponse
synchrone d'un defilement, et le lien vers le Compositor s'est ferme pendant
l'attente. WebContent, lui, VIT : il marque le Compositor perdu
(`CompositorConnection::did_lose_compositor`) et attend une reconnexion.

Le cycle de vie upstream (epingle cdfe5f8) :

  1. l'UI demande `connect_web_content()` ; le Compositor alloue un
     identifiant (`IDAllocator` : ALEATOIRE, et REUTILISE apres liberation),
     cree `ConnectionFromWebContent` et rend l'identifiant ;
  2. l'UI le met en cache (`WebContentClient::m_compositor_connection_id`)
     et l'envoie dans chaque `create_context(ctx, page, conn)` ;
  3. quand CETTE connexion meurt (pair ferme, `did_misbehave`, message
     malforme, erreur de transport), `on_death` la retire et LIBERE
     l'identifiant ... sans le dire a l'UI. Seule la mort du PROCESSUS
     Compositor declenche une reprise (`recover_compositor_process`).

L'UI garde donc un identifiant mort. Le `create_context` suivant -- une
nouvelle page, un cadre, un navigable cree par ce WebContent -- trouve
`connection == nullptr` : VERIFY, le Compositor tombe, et avec lui TOUS les
onglets. Pire, l'identifiant libere peut etre REALLOUE a un autre WebContent :
le contexte serait alors lie au mauvais processus, sans aucun crash.

Le correctif ne remplace pas le VERIFY par un `return` aveugle ; il ferme le
trou du protocole :

  * identifiants MONOTONES, jamais reutilises : un identifiant designe une
    seule connexion pour toute la vie du Compositor ;
  * `did_lose_web_content_connection(conn)` (nouveau message Compositor ->
    UI) a la mort de chaque connexion ;
  * l'UI retrouve le WebContent qui portait cet identifiant et, s'il vit,
    rejoue pour LUI SEUL la reprise que upstream fait pour tous a la mort du
    processus : reconnexion, recreation des contexts, etat des vues,
    `compositor_process_reconnected`. Au plus 3 reprises par processus ;
    au-dela, le WebContent est termine et la vue suit le chemin de crash
    normal (pas de boucle) ;
  * dans le Compositor, un `create_context` pour une connexion DEJA ALLOUEE
    puis morte est un message tardif LEGITIME : l'UI l'a envoye avant de
    lire `did_lose_web_content_connection`, qui croise. Il est journalise
    (`[LB] LATE_MESSAGE`) et ignore -- la reprise recree ce contexte, que
    l'UI a deja memorise. Un identifiant JAMAIS alloue reste une violation
    du protocole : VERIFY.
  * cote WebContent, la connexion remplacee est abandonnee proprement
    (`abandon_before_reconnect`) : la perte est signalee exactement une fois,
    AVANT la reconnexion, quel que soit l'ordre d'arrivee de l'EOF.

Journal structure (le banc et la campagne de convergence le lisent) :
  [LB] CONNECTION_CREATE / CONNECTION_REMOVE / PEER_CLOSE (Compositor)
  [LB] CONTEXT_CREATE / CONTEXT_DESTROY / LATE_MESSAGE      (Compositor)
  [LB] COMPOSITOR_LINK_LOST / _RECOVER / _RECOVERED / _GIVE_UP (UI)
  [LB] PROCESS_CREATE / PROCESS_EXIT (UI, ProcessManager ; BOUCHAUD_PROCESSUS_JOURNAL_V1)
  [LB] LINK_CUT_TEST                                        (WebContent, banc)

Banc : `debug_request("bouchaud-crash-rendu")` fait fauter le WebContent
(BOUCHAUD_CRASH_RENDU_V1).

Banc : `debug_request("bouchaud-coupe-lien-compositor")` ferme, depuis
WebContent, son lien vers le Compositor -- l'etat exact du journal
utilisateur. Seul le processus UI peut l'envoyer ; UI/Bouchaud ne le fait que
si le banc exporte `BOUCHAUD_LB_BANC_COUPE_LIEN`.

Ancres strictes, fail-closed, idempotent.
"""
import sys
from pathlib import Path

MARQUEUR = "BOUCHAUD_COMPOSITOR_LIEN_V1"


def remplace(chemin: Path, ancre: str, nouveau: str) -> None:
    texte = chemin.read_text(encoding="utf-8")
    if nouveau in texte:
        return
    if texte.count(ancre) != 1:
        raise SystemExit(f"compositor lien : ancre introuvable ou ambigue dans {chemin} :\n{ancre}")
    chemin.write_text(texte.replace(ancre, nouveau, 1), encoding="utf-8")


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: prepare-compositor-lien.py <arbre-ladybird>", file=sys.stderr)
        return 2
    racine = Path(sys.argv[1]).resolve()
    comp = racine / "Services/Compositor"
    wv = racine / "Libraries/LibWebView"
    wc = racine / "Services/WebContent"

    # --- IPC : Compositor -> UI -------------------------------------------
    remplace(
        comp / "CompositorControlClient.ipc",
        "    did_present_frame(Web::Compositor::CompositorContextId context_id, Gfx::IntRect content_rect, Gfx::IntRect damage_rect, i32 bitmap_id) =|\n",
        "    did_present_frame(Web::Compositor::CompositorContextId context_id, Gfx::IntRect content_rect, Gfx::IntRect damage_rect, i32 bitmap_id) =|\n"
        f"    // {MARQUEUR}\n"
        "    did_lose_web_content_connection(i32 web_content_connection_id) =|\n",
    )

    # --- Compositor : identifiants monotones, notification, message tardif --
    cfc = comp / "ConnectionFromClient.cpp"
    remplace(
        cfc,
        "#include <AK/IDAllocator.h>\n",
        "#include <AK/NumericLimits.h>\n",
    )
    remplace(
        cfc,
        "static IDAllocator s_web_content_connection_ids;\n",
        f"// {MARQUEUR}\n"
        "// Web content connection ids are MONOTONIC and never reused: an id names a single connection for the whole life\n"
        "// of the Compositor. The UI process caches it, and a reused id would bind a late create_context() to whichever\n"
        "// WebContent happened to receive it next.\n"
        "static i32 s_next_web_content_connection_id = 1;\n"
        "\n"
        "static bool web_content_connection_id_was_allocated(i32 web_content_connection_id)\n"
        "{\n"
        "    return web_content_connection_id > 0 && web_content_connection_id < s_next_web_content_connection_id;\n"
        "}\n",
    )
    remplace(
        cfc,
        "    auto web_content_connection_id = s_web_content_connection_ids.allocate();\n",
        "    VERIFY(s_next_web_content_connection_id < NumericLimits<i32>::max());\n"
        "    auto web_content_connection_id = s_next_web_content_connection_id++;\n",
    )
    remplace(
        cfc,
        "        m_web_content_connections.remove(client_id);\n"
        "        s_web_content_connection_ids.deallocate(client_id);\n"
        "    });\n"
        "    m_web_content_connections.set(web_content_connection_id, move(connection));\n",
        "        m_web_content_connections.remove(client_id);\n"
        "        dbgln(\"[LB] CONNECTION_REMOVE conn={} restantes={}\", client_id, m_web_content_connections.size());\n"
        "        // The UI process still holds this id. Tell it, so that it reconnects the WebContent if it is alive\n"
        "        // instead of sending create_context() for a connection that no longer exists.\n"
        "        if (is_open())\n"
        "            async_did_lose_web_content_connection(client_id);\n"
        "    });\n"
        "    m_web_content_connections.set(web_content_connection_id, move(connection));\n"
        "    dbgln(\"[LB] CONNECTION_CREATE conn={} total={}\", web_content_connection_id, m_web_content_connections.size());\n",
    )
    remplace(
        cfc,
        "    auto* connection = web_content_connection(web_content_connection_id);\n"
        "    VERIFY(connection);\n"
        "    m_compositor_state->create_context(context_id, page_id, *connection);\n",
        "    auto* connection = web_content_connection(web_content_connection_id);\n"
        "    if (!connection) {\n"
        "        // A connection that existed and died: did_lose_web_content_connection() is on its way to the UI process,\n"
        "        // which sent this before reading it. The UI has recorded the context and recreates it on the new\n"
        "        // connection. An id that was NEVER allocated is a protocol violation.\n"
        "        VERIFY(web_content_connection_id_was_allocated(web_content_connection_id));\n"
        "        dbgln(\"[LB] LATE_MESSAGE msg=create_context ctx={} page={} conn={} etat=connexion-morte\",\n"
        "            context_id.value(), page_id, web_content_connection_id);\n"
        "        return;\n"
        "    }\n"
        "    dbgln(\"[LB] CONTEXT_CREATE ctx={} page={} conn={}\", context_id.value(), page_id, web_content_connection_id);\n"
        "    m_compositor_state->create_context(context_id, page_id, *connection);\n",
    )

    cfw = comp / "ConnectionFromWebContent.cpp"
    remplace(
        cfw,
        "    auto protector = NonnullRefPtr { *this };\n"
        "    m_compositor_state->destroy_contexts_for_web_content_client(*this);\n",
        "    auto protector = NonnullRefPtr { *this };\n"
        f"    // {MARQUEUR}\n"
        "    dbgln(\"[LB] PEER_CLOSE conn={}\", client_id());\n"
        "    m_compositor_state->destroy_contexts_for_web_content_client(*this);\n",
    )
    remplace(
        cfw,
        "    if (!context_is_owned_by_this_connection(context_id))\n"
        "        return;\n"
        "    m_compositor_state->destroy_context(context_id);\n",
        "    if (!context_is_owned_by_this_connection(context_id))\n"
        "        return;\n"
        "    dbgln(\"[LB] CONTEXT_DESTROY ctx={} conn={}\", context_id.value(), client_id());\n"
        "    m_compositor_state->destroy_context(context_id);\n",
    )

    # --- UI : recevoir la perte, reprendre CE WebContent ----------------------
    remplace(
        wv / "CompositorClient.h",
        "    virtual void did_present_frame(Web::Compositor::CompositorContextId, Gfx::IntRect content_rect, Gfx::IntRect damage_rect, i32 bitmap_id) override;\n",
        "    virtual void did_present_frame(Web::Compositor::CompositorContextId, Gfx::IntRect content_rect, Gfx::IntRect damage_rect, i32 bitmap_id) override;\n"
        f"    // {MARQUEUR}\n"
        "    virtual void did_lose_web_content_connection(i32 web_content_connection_id) override;\n",
    )
    remplace(
        wv / "CompositorClient.cpp",
        "#include <LibCore/EventLoop.h>\n",
        "#include <LibCore/EventLoop.h>\n"
        "#include <LibWebView/Application.h>\n",
    )
    remplace(
        wv / "CompositorClient.cpp",
        "    async_presented_bitmap_ready_to_paint(context_id, bitmap_id);\n"
        "}\n",
        "    async_presented_bitmap_ready_to_paint(context_id, bitmap_id);\n"
        "}\n"
        "\n"
        f"// {MARQUEUR}\n"
        "void CompositorClient::did_lose_web_content_connection(i32 web_content_connection_id)\n"
        "{\n"
        "    // Out of the IPC handler: recovery issues synchronous requests to this same connection.\n"
        "    Core::deferred_invoke([web_content_connection_id] {\n"
        "        Application::the().did_lose_compositor_web_content_connection({}, web_content_connection_id);\n"
        "    });\n"
        "}\n",
    )
    remplace(
        wv / "Application.h",
        "    ErrorOr<void> try_register_compositor_context(WebContentClient&, Web::Compositor::CompositorContextId, Optional<u64> page_id);\n",
        "    ErrorOr<void> try_register_compositor_context(WebContentClient&, Web::Compositor::CompositorContextId, Optional<u64> page_id);\n"
        f"    // {MARQUEUR}\n"
        "    void did_lose_compositor_web_content_connection(Badge<CompositorClient>, i32 web_content_connection_id);\n",
    )
    remplace(
        wv / "Application.h",
        "    size_t m_compositor_restart_count { 0 };\n",
        "    size_t m_compositor_restart_count { 0 };\n"
        f"    // {MARQUEUR} : reprises du lien Compositor, par processus WebContent.\n"
        "    HashMap<pid_t, size_t> m_compositor_link_recoveries;\n",
    )
    remplace(
        wv / "Application.cpp",
        "ErrorOr<void> Application::launch_request_server()\n",
        f"// {MARQUEUR}\n"
        "// One WebContent's connection to the Compositor died while the Compositor process lives on. Replay, for that\n"
        "// WebContent only, what recover_compositor_process() does for all of them.\n"
        "void Application::did_lose_compositor_web_content_connection(Badge<CompositorClient>, i32 web_content_connection_id)\n"
        "{\n"
        "    if (Core::EventLoop::current().was_exit_requested() || !m_compositor_client)\n"
        "        return;\n"
        "    if (m_compositor_recovery_state != CompositorRecoveryState::Idle)\n"
        "        return;\n"
        "\n"
        "    RefPtr<WebContentClient> owner;\n"
        "    WebContentClient::for_each_client([&](WebContentClient& client) {\n"
        "        if (client.compositor_connection_id({}) == web_content_connection_id) {\n"
        "            owner = client;\n"
        "            return IterationDecision::Break;\n"
        "        }\n"
        "        return IterationDecision::Continue;\n"
        "    });\n"
        "\n"
        "    // Canvas-only clients (WebWorker processes) and WebContent processes that are already gone hold no id here.\n"
        "    if (!owner || !owner->is_open()) {\n"
        "        dbgln(\"[LB] COMPOSITOR_LINK_LOST conn={} proprietaire={}\", web_content_connection_id, owner ? \"ferme\"sv : \"aucun\"sv);\n"
        "        return;\n"
        "    }\n"
        "\n"
        "    auto pid = owner->pid();\n"
        "    auto& recoveries = m_compositor_link_recoveries.ensure(pid, []() -> size_t { return 0; });\n"
        "    constexpr size_t max_compositor_link_recoveries = 3;\n"
        "    dbgln(\"[LB] COMPOSITOR_LINK_LOST conn={} pid={} reprises={}\", web_content_connection_id, pid, recoveries);\n"
        "    if (recoveries >= max_compositor_link_recoveries) {\n"
        "        // Not a restart loop: hand the process to the normal WebContent crash path.\n"
        "        dbgln(\"[LB] COMPOSITOR_LINK_GIVE_UP pid={} reprises={}\", pid, recoveries);\n"
        "        (void)Core::System::kill(pid, SIGKILL);\n"
        "        return;\n"
        "    }\n"
        "    ++recoveries;\n"
        "\n"
        "    dbgln(\"[LB] COMPOSITOR_LINK_RECOVER pid={} ancienne={} tentative={}\", pid, web_content_connection_id, recoveries);\n"
        "    if (auto result = owner->reconnect_to_compositor_process({}); result.is_error()) {\n"
        "        dbgln(\"[LB] COMPOSITOR_LINK_GIVE_UP pid={} raison=reconnexion: {}\", pid, result.error());\n"
        "        (void)Core::System::kill(pid, SIGKILL);\n"
        "        return;\n"
        "    }\n"
        "    if (auto result = owner->recreate_compositor_contexts({}); result.is_error()) {\n"
        "        dbgln(\"[LB] COMPOSITOR_LINK_GIVE_UP pid={} raison=contextes: {}\", pid, result.error());\n"
        "        (void)Core::System::kill(pid, SIGKILL);\n"
        "        return;\n"
        "    }\n"
        "    owner->replay_compositor_view_state_after_reconnect({});\n"
        "    owner->notify_compositor_process_reconnected({});\n"
        "    dbgln(\"[LB] COMPOSITOR_LINK_RECOVERED pid={} conn={}\", pid, owner->compositor_connection_id({}));\n"
        "}\n"
        "\n"
        "ErrorOr<void> Application::launch_request_server()\n",
    )

    # --- WebContent : abandonner proprement le lien remplace -------------------
    remplace(
        wv / "CompositorConnection.h",
        "    Function<void()> on_compositor_lost;\n",
        "    Function<void()> on_compositor_lost;\n"
        "\n"
        f"    // {MARQUEUR}\n"
        "    // Called right before this connection is replaced by a new one: report the loss exactly once, now, then\n"
        "    // detach. A deferred EOF arriving later finds the loss already reported and the callbacks gone, so it can\n"
        "    // no longer tell the pages that the NEW connection was lost.\n"
        "    void abandon_before_reconnect();\n"
        "\n"
        "    // BOUCHAUD_LISTES_RETENUES_V1\n"
        "    // A replacement connection reaches a Compositor whose contexts for this WebContent are NEW and empty, and\n"
        "    // the UI registers them on its own schedule. Until compositor_process_reconnected() resets what the pages\n"
        "    // believe the Compositor holds, a display list or its deltas would be dropped (context not owned yet) or\n"
        "    // applied against an empty resource storage (font not found, VERIFY in DrawGlyphRun). Hold them.\n"
        "    void hold_display_lists_until_reconnected() { m_display_lists_held = true; }\n"
        "    void release_display_lists();\n",
    )
    remplace(
        wv / "CompositorConnection.h",
        "    bool m_has_lost_compositor { false };\n",
        "    bool m_has_lost_compositor { false };\n"
        f"    // {MARQUEUR} / BOUCHAUD_LISTES_RETENUES_V1\n"
        "    bool m_display_lists_held { false };\n"
        "    size_t m_display_list_messages_held { 0 };\n",
    )
    remplace(
        wv / "CompositorConnection.cpp",
        "bool CompositorConnection::can_send_message_to_compositor() const\n",
        f"// {MARQUEUR}\n"
        "void CompositorConnection::abandon_before_reconnect()\n"
        "{\n"
        "    if (is_open())\n"
        "        shutdown();\n"
        "    did_lose_compositor();\n"
        "    on_mouse_event = nullptr;\n"
        "    on_compositor_lost = nullptr;\n"
        "}\n"
        "\n"
        "// BOUCHAUD_LISTES_RETENUES_V1\n"
        "void CompositorConnection::release_display_lists()\n"
        "{\n"
        "    if (m_display_lists_held)\n"
        "        dbgln(\"[LB] DISPLAY_LISTS_RELEASED retenues={}\", m_display_list_messages_held);\n"
        "    m_display_lists_held = false;\n"
        "    m_display_list_messages_held = 0;\n"
        "}\n"
        "\n"
        "bool CompositorConnection::can_send_message_to_compositor() const\n",
    )
    for envoi in ("update_display_list", "update_visual_context_tree", "update_scroll_state"):
        texte_cc = (wv / "CompositorConnection.cpp").read_text(encoding="utf-8")
        debut = texte_cc.find(f"void CompositorConnection::{envoi}(")
        if debut < 0:
            raise SystemExit(f"compositor lien : {envoi} introuvable")
        garde = "{\n    if (!can_send_message_to_compositor())\n        return;\n"
        position = texte_cc.find(garde, debut)
        if position < 0 or texte_cc.find("\n}\n", debut) < position:
            raise SystemExit(f"compositor lien : garde de {envoi} introuvable")
        retenue = (
            "{\n    if (!can_send_message_to_compositor())\n        return;\n"
            f"    if (m_display_lists_held) {{ // BOUCHAUD_LISTES_RETENUES_V1 ({envoi})\n"
            "        ++m_display_list_messages_held;\n"
            "        return;\n"
            "    }\n"
        )
        if texte_cc[position:position + len(retenue)] != retenue:
            texte_cc = texte_cc[:position] + retenue + texte_cc[position + len(garde):]
            (wv / "CompositorConnection.cpp").write_text(texte_cc, encoding="utf-8")
    cfcw = wc / "ConnectionFromClient.cpp"
    remplace(
        cfcw,
        "    auto transport = MUST(handle.create_transport());\n"
        "    m_compositor_connection = adopt_ref(*new WebView::CompositorConnection(move(transport)));\n",
        "    auto transport = MUST(handle.create_transport());\n"
        f"    // {MARQUEUR}\n"
        "    bool const is_replacement = !m_compositor_connection.is_null();\n"
        "    if (m_compositor_connection)\n"
        "        m_compositor_connection->abandon_before_reconnect();\n"
        "    m_compositor_connection = adopt_ref(*new WebView::CompositorConnection(move(transport)));\n",
    )
    remplace(
        cfcw,
        "    m_compositor_connection->ensure_video_presentation_channel();\n}\n",
        "    m_compositor_connection->ensure_video_presentation_channel();\n"
        "\n"
        "    // BOUCHAUD_LISTES_RETENUES_V1 : the first connection starts empty on both sides; a replacement does not.\n"
        "    if (is_replacement) {\n"
        "        dbgln(\"[LB] DISPLAY_LISTS_HELD raison=connexion_remplacee\");\n"
        "        m_compositor_connection->hold_display_lists_until_reconnected();\n"
        "    }\n"
        "}\n",
    )
    remplace(
        cfcw,
        "void ConnectionFromClient::compositor_process_reconnected()\n{\n    m_page_host->compositor_process_reconnected();\n}\n",
        "void ConnectionFromClient::compositor_process_reconnected()\n{\n"
        "    // BOUCHAUD_LISTES_RETENUES_V1 : the UI has registered the new contexts; the pages now forget what the old\n"
        "    // Compositor state held (repaint_after_compositor_process_reconnect) and record full transactions.\n"
        "    if (m_compositor_connection)\n"
        "        m_compositor_connection->release_display_lists();\n"
        "    m_page_host->compositor_process_reconnected();\n}\n",
    )
    remplace(
        cfcw,
        "    if (request == \"dump-session-history\") {\n",
        f"    // {MARQUEUR} -- banc : fermer, depuis WebContent, le lien vers le Compositor (etat du crash observe).\n"
        "    if (request == \"bouchaud-coupe-lien-compositor\") {\n"
        "        dbgln(\"[LB] LINK_CUT_TEST page={} lien={}\", page_id, m_compositor_connection && m_compositor_connection->is_open());\n"
        "        if (m_compositor_connection && m_compositor_connection->is_open())\n"
        "            m_compositor_connection->shutdown();\n"
        "        return;\n"
        "    }\n"
        "\n"
        "    if (request == \"dump-session-history\") {\n",
    )
    # BOUCHAUD_CRASH_RENDU_V1 -- banc : une VRAIE faute dans ce WebContent
    # (ecriture a une page non mappee). Le noyau la livre en SIGSEGV, le
    # processus meurt, ses sockets se ferment : exactement un rendu qui
    # plante. Le banc prouve que les autres onglets et le Compositor vivent.
    remplace(
        cfcw,
        "    if (request == \"bouchaud-coupe-lien-compositor\") {\n",
        "    if (request == \"bouchaud-crash-rendu\") {\n"
        "        dbgln(\"[LB] RENDERER_CRASH_TEST page={}\", page_id);\n"
        "        FlatPtr volatile adresse = 0x10;\n"
        "        *reinterpret_cast<int volatile*>(adresse) = 1;\n"
        "        return;\n"
        "    }\n"
        "\n"
        "    if (request == \"bouchaud-coupe-lien-compositor\") {\n",
    )
    # BOUCHAUD_PROCESSUS_JOURNAL_V1 -- cycle de vie des processus du
    # navigateur, vu par le gestionnaire d'upstream (le seul endroit ou
    # chaque service est enregistre et chaque mort recoltee).
    pm = wv / "ProcessManager.cpp"
    # BOUCHAUD_COMPOSITOR_PREUVE_V1 : compter dans le gestionnaire, pas dans
    # un grep de PERF_EXECVE. Le run 37776506426 a perdu ce marqueur par
    # entrelacement serie. Les compteurs persistent meme si une ligne se perd.
    remplace(
        wv / "ProcessManager.h",
        "    Core::Platform::ProcessStatistics m_statistics;\n",
        "    u64 m_bouchaud_compositor_created { 0 };\n"
        "    u64 m_bouchaud_compositor_removed { 0 };\n"
        "    u64 m_bouchaud_compositor_sample { 0 };\n"
        "    RefPtr<Core::Timer> m_bouchaud_compositor_timer;\n"
        "    Core::Platform::ProcessStatistics m_statistics;\n",
    )
    remplace(pm, "#include <AK/String.h>\n", "#include <AK/String.h>\n#include <stdlib.h>\n")
    remplace(
        pm,
        "    add_process(Process(WebView::ProcessType::Browser, nullptr, Core::Process::current()));\n",
        "    add_process(Process(WebView::ProcessType::Browser, nullptr, Core::Process::current()));\n"
        "    if (getenv(\"BOUCHAUD_LB_LIFECYCLE_PROOF\")) {\n"
        "        m_bouchaud_compositor_timer = Core::Timer::create_repeating(1000, [this] {\n"
        "            size_t live = 0;\n"
        "            pid_t pid = 0;\n"
        "            for_each_process([&](Process& process) {\n"
        "                if (process.type() == ProcessType::Compositor) {\n"
        "                    ++live;\n"
        "                    pid = process.pid();\n"
        "                }\n"
        "            });\n"
        "            dbgln(\"[LB:COMPOSITOR_STATE] seq={} created={} removed={} live={} pid={} END\",\n"
        "                ++m_bouchaud_compositor_sample, m_bouchaud_compositor_created,\n"
        "                m_bouchaud_compositor_removed, live, pid);\n"
        "        });\n"
        "        m_bouchaud_compositor_timer->start();\n"
        "    }\n",
    )
    remplace(
        pm,
        "    return m_processes.take(pid);\n",
        "    auto process = m_processes.take(pid);\n"
        "    if (process.has_value() && process->type() == ProcessType::Compositor)\n"
        "        ++m_bouchaud_compositor_removed;\n"
        "    return process;\n",
    )
    remplace(
        pm,
        "    auto pid = process.pid();\n"
        "    on_process_added(process);\n",
        "    auto pid = process.pid();\n"
        "    if (process.type() == ProcessType::Compositor)\n"
        "        ++m_bouchaud_compositor_created;\n"
        "    dbgln(\"[LB] PROCESS_CREATE type={} pid={} total={}\", process_name_from_type(process.type()), pid, m_processes.size() + 1);\n"
        "    on_process_added(process);\n",
    )
    remplace(
        pm,
        "        if (auto process = remove_process(pid); process.has_value())\n"
        "            on_process_exited(process.release_value(), exit_status);\n",
        "        if (auto process = remove_process(pid); process.has_value()) {\n"
        "            dbgln(\"[LB] PROCESS_EXIT type={} pid={} statut={} restants={}\", process_name_from_type(process->type()), pid,\n"
        "                exit_status.value_or(-1), m_processes.size());\n"
        "            on_process_exited(process.release_value(), exit_status);\n"
        "        }\n",
    )
    # BOUCHAUD_PROCESSUS_JOURNAL_V1 -- ce que voit ProcessMonitor : chaque
    # SIGCHLD recu et ce que waitpid rend. Run 37584587000 : un WebContent
    # mort (faute) n'a JAMAIS ete vu par le navigateur, alors que le noyau
    # livre SIGCHLD et waitpid a un pere multi-fils
    # (sigchld-multifil-probe, SIGCHLD_MULTIFIL_OK). Ces lignes disent lequel
    # des maillons manque.
    pmon = wv / "ProcessMonitor.cpp"
    remplace(
        pmon,
        "    m_signal_handle = Core::EventLoop::register_signal(SIGCHLD, [this](int) {\n"
        "        auto result = Core::System::waitpid(-1, WNOHANG);\n"
        "        while (!result.is_error() && result.value().pid > 0) {\n"
        "            auto& [pid, status] = result.value();\n",
        "    m_signal_handle = Core::EventLoop::register_signal(SIGCHLD, [this](int) {\n"
        "        auto result = Core::System::waitpid(-1, WNOHANG);\n"
        "        dbgln(\"[LB] SIGCHLD_RECU waitpid={}\", result.is_error() ? -1 : result.value().pid);\n"
        "        while (!result.is_error() && result.value().pid > 0) {\n"
        "            auto& [pid, status] = result.value();\n"
        "            dbgln(\"[LB] SIGCHLD_FILS pid={} surveille={} signale={} signal={} sorti={} code={}\", pid, m_monitored_processes.contains(pid),\n"
        "                WIFSIGNALED(status), WIFSIGNALED(status) ? WTERMSIG(status) : 0, WIFEXITED(status), WIFEXITED(status) ? WEXITSTATUS(status) : 0);\n",
    )
    # BOUCHAUD_OOPIF_V1 -- isolation des cadres : quel processus heberge un
    # cadre d'un autre site. Sans cette ligne, le banc ne voit qu'un
    # WebContent de plus, sans savoir pour qui.
    sim = wv / "SiteIsolationManager.cpp"
    remplace(
        sim,
        "    child_frame->set_remote_host(move(remote_client), remote_page_id);\n",
        "    dbgln(\"[LB] OOPIF_REMOTE parent_pid={} hote_pid={} page={} page_distante={}\", parent_client.pid(), remote_client->pid(), page_id, remote_page_id);\n"
        "    child_frame->set_remote_host(move(remote_client), remote_page_id);\n",
    )
    # BOUCHAUD_SIGNAL_BOUCLE_V1 -- un signal va a la boucle d'evenements du
    # fil qui a ENREGISTRE son gestionnaire, pas a celle du fil qui le recoit.
    # Upstream ecrit dans le tube du fil RECEVEUR et abandonne le signal si ce
    # fil n'a pas de boucle : sous Linux le noyau livre au fil principal, sur
    # Bouchaud le premier fil qui repasse en mode utilisateur le prenait (run
    # 37589903681 : aucun [LB] SIGCHLD_RECU, aucun service mort recolte). Le
    # noyau donne desormais la preference au fil principal
    # (BOUCHAUD_SIGNAL_FIL_PRINCIPAL_V1) ; ceci couvre le cas ou il calcule
    # en mode utilisateur, et evite qu'un gestionnaire tourne sur une boucle
    # etrangere (ProcessManager::verify_event_loop).
    elu = racine / "Libraries/LibCore/EventLoopImplementationUnix.cpp"
    # Le gestionnaire de signal ne touche que des atomiques sans verrou et
    # n'appelle que write() et getpid() (surs en contexte de signal). La
    # destruction d'une boucle attend qu'aucun gestionnaire ne soit en vol vers
    # son tube avant de le fermer : sinon un fil pouvait lire le proprietaire,
    # perdre la main, et ecrire dans un tube ferme -- ou dans le descripteur
    # qu'un autre fil venait de rouvrir sous le meme numero.
    remplace(
        elu,
        "#include <AK/HashMap.h>\n",
        "#include <AK/Atomic.h>\n"
        "#include <AK/HashMap.h>\n",
    )
    remplace(
        elu,
        "#include <pthread.h>\n",
        "#include <errno.h>\n"
        "#include <pthread.h>\n"
        "#include <sched.h>\n",
    )
    remplace(
        elu,
        "thread_local ThreadData* s_this_thread_data;\n",
        "thread_local ThreadData* s_this_thread_data;\n"
        "// BOUCHAUD_SIGNAL_BOUCLE_V1 : la boucle proprietaire de chaque signal.\n"
        "struct SignalOwner {\n"
        "    Atomic<ThreadData*> owner { nullptr };\n"
        "    Atomic<int> wake_fd { -1 };\n"
        "    Atomic<pid_t> pid { 0 };\n"
        "    Atomic<int> in_flight { 0 };\n"
        "};\n"
        "static SignalOwner s_signal_owner[65];\n",
    )
    remplace(
        elu,
        "static void destroy_thread_data(void* value)\n"
        "{\n"
        "    s_this_thread_data = nullptr;\n",
        "static void destroy_thread_data(void* value)\n"
        "{\n"
        "    // BOUCHAUD_SIGNAL_BOUCLE_V1 : plus de proprietaire mort, et plus aucun\n"
        "    // gestionnaire en vol vers son tube quand le destructeur le ferme.\n"
        "    // Ordre (seq_cst) : wake_fd = -1 PUIS lecture de in_flight ; le\n"
        "    // gestionnaire fait in_flight++ PUIS lit wake_fd. L'un des deux voit\n"
        "    // l'autre : soit le gestionnaire lit -1, soit on l'attend.\n"
        "    for (auto& slot : s_signal_owner) {\n"
        "        ThreadData* attendu = static_cast<ThreadData*>(value);\n"
        "        if (slot.owner.compare_exchange_strong(attendu, nullptr)) {\n"
        "            slot.wake_fd.store(-1);\n"
        "            while (slot.in_flight.load() != 0)\n"
        "                sched_yield();\n"
        "        }\n"
        "    }\n"
        "    s_this_thread_data = nullptr;\n",
    )
    remplace(
        elu,
        "    if (!s_this_thread_data)\n"
        "        return;\n"
        "    auto& thread_data = *s_this_thread_data;\n",
        "    // BOUCHAUD_SIGNAL_BOUCLE_V1 : vers la boucle qui a enregistre le\n"
        "    // gestionnaire, quel que soit le fil qui recoit le signal. Le pid\n"
        "    // ecarte la fenetre fork()/exec() (le tube est partage avec le pere).\n"
        "    if (signal_number > 0 && signal_number < 65) {\n"
        "        auto& slot = s_signal_owner[signal_number];\n"
        "        slot.in_flight.fetch_add(1);\n"
        "        int fd = slot.wake_fd.load();\n"
        "        bool livre = false;\n"
        "        if (fd >= 0 && slot.pid.load() == getpid()) {\n"
        "            int saved_errno = errno;\n"
        "            livre = write(fd, &signal_number, sizeof(signal_number)) == sizeof(signal_number);\n"
        "            errno = saved_errno;\n"
        "        }\n"
        "        slot.in_flight.fetch_sub(1);\n"
        "        if (livre)\n"
        "            return;\n"
        "    }\n"
        "    // Pas de proprietaire (ou il est mort) : comportement amont, le fil\n"
        "    // receveur.\n"
        "    if (!s_this_thread_data)\n"
        "        return;\n"
        "    auto& thread_data = *s_this_thread_data;\n",
    )
    remplace(
        elu,
        "    if (remove_signal_number != 0)\n"
        "        info.signal_handlers.remove(remove_signal_number);\n",
        "    if (remove_signal_number != 0) {\n"
        "        info.signal_handlers.remove(remove_signal_number);\n"
        "        // BOUCHAUD_SIGNAL_BOUCLE_V1 : plus de gestionnaire, plus de\n"
        "        // proprietaire. Le tube reste ouvert (son fil vit) : rien a attendre.\n"
        "        if (remove_signal_number > 0 && remove_signal_number < 65) {\n"
        "            auto& slot = s_signal_owner[remove_signal_number];\n"
        "            slot.wake_fd.store(-1);\n"
        "            slot.owner.store(nullptr);\n"
        "        }\n"
        "    }\n",
    )
    remplace(
        elu,
        "int EventLoopManagerUnix::register_signal(int signal_number, Function<void(int)> handler)\n"
        "{\n"
        "    VERIFY(signal_number != 0);\n",
        "int EventLoopManagerUnix::register_signal(int signal_number, Function<void(int)> handler)\n"
        "{\n"
        "    VERIFY(signal_number != 0);\n"
        "    // BOUCHAUD_SIGNAL_BOUCLE_V1 : le premier fil qui enregistre ce signal\n"
        "    // en possede la boucle de distribution, jusqu'au retrait du dernier\n"
        "    // gestionnaire ou a la mort de ce fil. pid avant tube : qui lit le\n"
        "    // tube lit aussi le bon pid.\n"
        "    if (signal_number > 0 && signal_number < 65) {\n"
        "        auto& slot = s_signal_owner[signal_number];\n"
        "        auto& self = ThreadData::the();\n"
        "        ThreadData* attendu = nullptr;\n"
        "        if (slot.owner.compare_exchange_strong(attendu, &self)) {\n"
        "            slot.pid.store(self.pid);\n"
        "            slot.wake_fd.store(self.wake_pipe_fds[1]);\n"
        "        }\n"
        "    }\n",
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
