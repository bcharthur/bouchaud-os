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
  [LB] LINK_CUT_TEST                                        (WebContent, banc)

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
        "    void abandon_before_reconnect();\n",
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
        "bool CompositorConnection::can_send_message_to_compositor() const\n",
    )
    cfcw = wc / "ConnectionFromClient.cpp"
    remplace(
        cfcw,
        "    auto transport = MUST(handle.create_transport());\n"
        "    m_compositor_connection = adopt_ref(*new WebView::CompositorConnection(move(transport)));\n",
        "    auto transport = MUST(handle.create_transport());\n"
        f"    // {MARQUEUR}\n"
        "    if (m_compositor_connection)\n"
        "        m_compositor_connection->abandon_before_reconnect();\n"
        "    m_compositor_connection = adopt_ref(*new WebView::CompositorConnection(move(transport)));\n",
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
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
