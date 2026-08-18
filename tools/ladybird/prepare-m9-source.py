#!/usr/bin/env python3
# M9 adaptations for the disposable pinned Ladybird worktree.
# Runs AFTER prepare-browser-source.py so M8 remains intact.

from pathlib import Path
import sys

if len(sys.argv) != 2:
    raise SystemExit("usage: prepare-m9-source.py <ladybird-worktree>")

root = Path(sys.argv[1])


def replace_once(path: Path, old: str, new: str, label: str):
    data = path.read_text()
    if new in data:
        return
    if old not in data:
        raise SystemExit(f"M9 pattern not found ({label}) in {path}")
    path.write_text(data.replace(old, new, 1))


# RequestServer: inherited Bouchaud AF_UNIX socket.
request_main = root / "Services/RequestServer/main.cpp"
data = request_main.read_text()

if "BOUCHAUD_REQUESTSERVER_FD" not in data:
    data = data.replace(
        "#include <LibCore/EventLoop.h>\n",
        "#include <LibCore/EventLoop.h>\n"
        "#include <LibCore/Socket.h>\n"
        "#include <LibIPC/Transport.h>\n"
        "#include <cstdlib>\n",
        1,
    )

    old = '''    auto client = TRY(IPC::take_over_accepted_client_from_system_server<RequestServer::ConnectionFromClient>(
        mach_server_name,
        RequestServer::ConnectionFromClient::IsPrimaryConnection::Yes,
        RequestServer::IsPrivate::No,
        connections,
        disk_cache,
        LexicalPath::join(cache_path, "alt-svc-cache.txt"sv).string()));

    return event_loop.exec();'''

    new = '''#if defined(BOUCHAUD_PORT)
    if (auto* inherited_fd = getenv("BOUCHAUD_REQUESTSERVER_FD")) {
        auto fd = atoi(inherited_fd);
        if (fd < 0) {
            warnln("[ladybird-bouchaud] M9_REQUESTSERVER_BAD_FD {}", inherited_fd);
            return 64;
        }

        auto socket = TRY(Core::LocalSocket::adopt_fd(fd));
        auto client = RequestServer::ConnectionFromClient::construct(
            make<IPC::Transport>(move(socket)),
            RequestServer::ConnectionFromClient::IsPrimaryConnection::Yes,
            RequestServer::IsPrivate::No,
            connections,
            disk_cache,
            LexicalPath::join(cache_path, "alt-svc-cache.txt"sv).string());

        outln("[ladybird-bouchaud] M9_REQUESTSERVER_READY pid={} fd={}", Core::System::getpid(), fd);
        return event_loop.exec();
    }
#endif

    auto client = TRY(IPC::take_over_accepted_client_from_system_server<RequestServer::ConnectionFromClient>(
        mach_server_name,
        RequestServer::ConnectionFromClient::IsPrimaryConnection::Yes,
        RequestServer::IsPrivate::No,
        connections,
        disk_cache,
        LexicalPath::join(cache_path, "alt-svc-cache.txt"sv).string()));

    return event_loop.exec();'''

    if old not in data:
        raise SystemExit("M9 RequestServer takeover block changed upstream")
    request_main.write_text(data.replace(old, new, 1))


# WebContent: connect ResourceLoader directly to inherited RequestServer fd.
web_main = root / "Services/WebContent/main.cpp"
data = web_main.read_text()

if "BOUCHAUD_REQUEST_FD" not in data:
    anchor = '''    auto& heap = Web::Bindings::main_thread_vm().heap();
    webcontent_client->on_request_server_connection = [&heap](auto const& handle) {'''

    replacement = '''    auto& heap = Web::Bindings::main_thread_vm().heap();

#if defined(BOUCHAUD_PORT)
    if (getenv("BOUCHAUD_M9")) {
        auto* inherited_request_fd = getenv("BOUCHAUD_REQUEST_FD");
        if (!inherited_request_fd) {
            warnln("[ladybird-bouchaud] M9_REQUEST_FD_ABSENT");
            return 65;
        }

        auto request_fd = atoi(inherited_request_fd);
        if (request_fd < 0) {
            warnln("[ladybird-bouchaud] M9_REQUEST_FD_INVALID {}", inherited_request_fd);
            return 65;
        }

        auto request_socket = TRY(Core::LocalSocket::adopt_fd(request_fd));
        auto request_client = TRY(try_make_ref_counted<Requests::RequestClient>(
            make<IPC::Transport>(move(request_socket))));

        // DNS. Ladybird n'utilise pas `getaddrinfo` : il embarque son propre
        // resolveur (LibDNS), et `use_dns_over_tls` y vaut **true** par defaut.
        // Sans configuration explicite, la premiere resolution voudrait donc
        // ouvrir une session TLS vers un resolveur public — c'est-a-dire faire
        // dependre le DNS d'un magasin d'autorites, et la resolution du nom du
        // resolveur d'une resolution. Deux cercles pour rien.
        //
        // On lui designe donc un serveur par son **adresse**, en UDP simple.
        // `set_dns_server` accepte une IP litterale sans rien resoudre.
        // `10.0.2.3` est le resolveur du NAT de QEMU ; sur une machine reelle,
        // `BOUCHAUD_DNS_SERVER` prend le relais.
        char const* dns_server = getenv("BOUCHAUD_DNS_SERVER");
        if (dns_server == nullptr || *dns_server == '\\0')
            dns_server = "10.0.2.3";
        // Le parametre genere est un `StringView` : on nomme la chaine pour que
        // sa duree de vie couvre l'appel sans dependre d'un temporaire.
        auto dns_server_string = ByteString { dns_server };
        request_client->async_set_dns_server(dns_server_string.view(), 53, false, false);
        outln("[ladybird-bouchaud] M12_DNS_SERVER {} port=53 tls=false", dns_server);

        if (Web::ResourceLoader::is_initialized())
            Web::ResourceLoader::the().set_client(move(request_client));
        else
            Web::ResourceLoader::initialize(heap, move(request_client));

        outln("[ladybird-bouchaud] M9_REQUESTSERVER_CONNECTED pid={} fd={}", Core::System::getpid(), request_fd);
    }
#endif
    webcontent_client->on_request_server_connection = [&heap](auto const& handle) {'''

    if anchor not in data:
        raise SystemExit("M9 WebContent ResourceLoader anchor changed")
    data = data.replace(anchor, replacement, 1)

    old_start = '''#if defined(BOUCHAUD_PORT)
    if (getenv("BOUCHAUD_M8"))
        webcontent_client->bouchaud_m8_start();
#endif'''

    new_start = '''#if defined(BOUCHAUD_PORT)
    if (getenv("BOUCHAUD_M8"))
        webcontent_client->bouchaud_m8_start();
    else if (getenv("BOUCHAUD_M9"))
        webcontent_client->bouchaud_m9_start();
#endif'''

    if old_start not in data:
        raise SystemExit("M9 WebContent bootstrap start anchor changed")
    data = data.replace(old_start, new_start, 1)
    web_main.write_text(data)


# ConnectionFromClient: M9 start -> normal Page::load navigation.
connection_h = root / "Services/WebContent/ConnectionFromClient.h"
replace_once(
    connection_h,
    '''#if defined(BOUCHAUD_PORT)
    void bouchaud_m8_start();
#endif''',
    '''#if defined(BOUCHAUD_PORT)
    void bouchaud_m8_start();
    void bouchaud_m9_start();
#endif''',
    "ConnectionFromClient M9 declaration",
)

connection_cpp = root / "Services/WebContent/ConnectionFromClient.cpp"
data = connection_cpp.read_text()

if "void ConnectionFromClient::bouchaud_m9_start()" not in data:
    old_tail = '''    load_html(page_id, ByteString { html });
}
#endif
'''

    new_tail = '''    load_html(page_id, ByteString { html });
}

void ConnectionFromClient::bouchaud_m9_start()
{
    constexpr u64 page_id = 1;
    auto width = bouchaud_env_positive_int("BO_SURFACE_WIDTH", 1100);
    auto height = bouchaud_env_positive_int("BO_SURFACE_HEIGHT", 604);

    Web::HTML::CrossProcessId root_navigable_id { .namespace_id = 1, .local_id = 1 };
    Web::HTML::CrossProcessIdAllocator allocator { .namespace_id = 1, .next_local_id = 2 };

    // Browser normally sends the system font family before creating the first
    // Page. Bouchaud has no Browser process at M9, so mirror the validated M8
    // bootstrap explicitly. Without this, StyleComputer dereferences the null
    // RefPtr returned by FontPlugin::default_font() during PageHost::initialize.
    set_system_font_family("SerenitySans"_string);
    auto m9_default_font = Web::Platform::FontPlugin::the().default_font(16);
    if (!m9_default_font) {
        warnln("[ladybird-bouchaud] M9_FONT_MISSING family=SerenitySans resource_root=/usr/share/ladybird");
        Core::Process::terminate_immediately(71);
    }
    outln("[ladybird-bouchaud] M9_FONT_READY family=SerenitySans");
    outln("[ladybird-bouchaud] M9_STAGE initialize begin");
    initialize(page_id, root_navigable_id, allocator);
    outln("[ladybird-bouchaud] M9_STAGE initialize ok");

    auto viewport = Gfx::IntSize { width, height }.to_type<Web::DevicePixels>();
    auto screen = Gfx::IntRect { 0, 0, width, height }.to_type<Web::DevicePixels>();
    update_screen_rects(page_id, Vector<Web::DevicePixelRect> { screen }, 0);
    set_viewport(page_id, viewport, 1.0, Web::ViewportIsFullscreen::No);
    set_window_size(page_id, viewport);
    set_has_focus(page_id, true);
    set_system_visibility_state(page_id, Web::HTML::VisibilityState::Visible);

    char const* requested_url = getenv("BOUCHAUD_M9_URL");
    if (!requested_url || !*requested_url)
        requested_url = "http://10.0.2.2:18080/m9.html";

    auto url = URL::create_with_url_or_path(ByteString { requested_url });
    if (!url.has_value()) {
        warnln("[ladybird-bouchaud] M9_URL_INVALID {}", requested_url);
        return;
    }

    outln("[ladybird-bouchaud] M9_BOOTSTRAP page={} viewport={}x{}", page_id, width, height);
    outln("[ladybird-bouchaud] M9_NAVIGATION_BEGIN url={}", *url);

    // Normal LibWeb path:
    // Page::load -> Navigable::navigate -> Fetch -> ResourceLoader
    // -> Requests::RequestClient -> RequestServer.
    load_url(page_id, *url, Web::Bindings::NavigationHistoryBehavior::Auto);
}
#endif
'''

    if old_tail not in data:
        raise SystemExit("M9 ConnectionFromClient M8 tail changed")
    connection_cpp.write_text(data.replace(old_tail, new_tail, 1))



# --- Sondes du chemin retour RequestServer -> WebContent ----------------------
#
# Le run precedent a etabli que le GET part, que la fixture repond 200, et que
# la navigation n'est ni commitee ni annulee : elle se tait. Le trou est donc
# dans les messages **retour**.
#
# Ce chemin n'est pas un simple flux d'octets sur la socket IPC : RequestServer
# cree une paire de sockets (`RequestPipe`), en passe le bout lecteur a
# WebContent par SCM_RIGHTS, et y ecrit le corps. Deux messages jalonnent
# l'echange — `request_started`, qui porte le descripteur, puis
# `request_finished`. Savoir lequel arrive et lequel manque partage le probleme
# en deux moities dont une seule restera a explorer.
requests_cpp = root / "Libraries/LibRequests/RequestClient.cpp"
data = requests_cpp.read_text()

if "bouchaud_m9_trace" not in data:
    anchor = "namespace Requests {\n"
    if anchor not in data:
        raise SystemExit("M9 RequestClient: namespace Requests introuvable")
    helper = anchor + "\nstatic bool bouchaud_m9_trace()\n{\n    return getenv(\"BOUCHAUD_M9\") != nullptr;\n}\n"
    data = data.replace(anchor, helper, 1)
    # `getenv` et `outln`. Le fichier utilise deja `warnln`, donc AK/Format.h
    # arrive transitivement — mais une inclusion transitive est une dependance
    # que personne n'a ecrite et que rien ne garantit.
    if "<cstdlib>" not in data:
        first_include = data.index("#include ")
        data = data[:first_include] + "#include <cstdlib>\n#include <AK/Format.h>\n" + data[first_include:]

    sondes = [
        ("void RequestClient::request_started(u64 request_id, IPC::File response_file)\n{\n",
         "M9_RS_REQUEST_STARTED id={}", "request_id"),
        ("void RequestClient::request_finished(u64 request_id, u64 total_size, RequestTimingInfo timing_info, Optional<NetworkError> network_error)\n{\n",
         "M9_RS_REQUEST_FINISHED id={} taille={}", "request_id, total_size"),
        # Le message central du trio. Le corps peut traverser entierement sans
        # que LibWeb n'ait de reponse a commiter : c'est ici que le code de
        # statut et les en-tetes arrivent, et sans eux il n'y a pas de document.
        ("void RequestClient::headers_became_available(u64 request_id, Vector<HTTP::Header> response_headers, Optional<u32> status_code, Optional<String> reason_phrase, Optional<IPC::File> javascript_bytecode_file, u64 javascript_bytecode_size, Optional<u64> javascript_bytecode_cache_vary_key, CameFromCache came_from_cache)\n{\n",
         "M9_RS_HEADERS id={} statut={} nb={}", "request_id, status_code.value_or(0), response_headers.size()"),
    ]
    for signature, message, args in sondes:
        if signature not in data:
            raise SystemExit("M9 RequestClient: signature introuvable -> " + message)
        data = data.replace(
            signature,
            signature + "    if (bouchaud_m9_trace())\n        outln(\"[ladybird-bouchaud] " + message + "\", " + args + ");\n",
            1)

    requests_cpp.write_text(data)


# Bouchaud M9 receives the response through a RequestPipe socketpair. The
# RequestServer marker above proves that the complete body has already been
# written to that pipe before `request_finished` reaches WebContent. On the
# current Bouchaud event loop the read notifier can nevertheless remain asleep,
# leaving Fetch waiting forever with the payload already queued. Drain the
# ready pipe once when completion is announced. This keeps the normal LibWeb
# streaming path and only compensates for the missing notifier wake-up on the
# Bouchaud M9 port; other platforms and M8 stay byte-for-byte untouched.
request_cpp = root / "Libraries/LibRequests/Request.cpp"
data = request_cpp.read_text()

if "M9_BODY_DRAIN_BEGIN" not in data:
    old = '''void Request::did_finish(Badge<RequestClient>, u64 total_size, RequestTimingInfo const& timing_info, Optional<NetworkError> const& network_error)
{
    auto effective_network_error = m_body_delivery_error.has_value() ? m_body_delivery_error : network_error;
    if (on_finish)
        on_finish(total_size, timing_info, effective_network_error);
}'''
    new = '''void Request::did_finish(Badge<RequestClient>, u64 total_size, RequestTimingInfo const& timing_info, Optional<NetworkError> const& network_error)
{
    auto effective_network_error = m_body_delivery_error.has_value() ? m_body_delivery_error : network_error;
#if defined(BOUCHAUD_PORT)
    if (getenv("BOUCHAUD_M9") != nullptr && m_internal_stream_data && m_internal_stream_data->read_notifier && m_internal_stream_data->read_notifier->on_activation) {
        outln("[ladybird-bouchaud] M9_BODY_DRAIN_BEGIN total={}", total_size);
        m_internal_stream_data->read_notifier->on_activation();
        outln("[ladybird-bouchaud] M9_BODY_DRAIN_DONE total={}", total_size);
    }
#endif
    if (on_finish)
        on_finish(total_size, timing_info, effective_network_error);
}'''
    if old not in data:
        raise SystemExit("M9 Request body completion hook changed upstream")
    data = data.replace(old, new, 1)
    if "<cstdlib>" not in data:
        first_include = data.index("#include ")
        data = data[:first_include] + "#include <cstdlib>\n#include <AK/Format.h>\n" + data[first_include:]
    request_cpp.write_text(data)


# --- Sonde des appels IPC synchrones -----------------------------------------
#
# `ConnectionBase::wait_for_specific_endpoint_message_impl` est la boucle
# d'attente d'un `send_sync`. Elle n'a **aucun delai d'expiration** : un message
# synchrone auquel personne ne repond bloque pour toujours.
#
# C'est le point de sonde qui nomme le coupable. Une navigation `http://`
# declenche des appels synchrones vers l'interface navigateur que `load_html`
# ne declenche jamais — cookies, HSTS, stockage — et le peer M9 n'est pas une
# interface complete. Le script court-circuite deja `decide_navigation_process`
# pour cette raison exacte ; s'il en reste d'autres, ils s'annonceront ici.
#
# La sonde affiche le message **avant** d'attendre : celui qui ne revient jamais
# est donc le dernier imprime.
ipc_cpp = root / "Libraries/LibIPC/Connection.cpp"
data = ipc_cpp.read_text()

if "M9_IPC_SYNC_WAIT" not in data:
    anchor = ("OwnPtr<IPC::Message> ConnectionBase::wait_for_specific_endpoint_message_impl(u32 endpoint_magic, int message_id)\n"
              "{\n")
    if anchor not in data:
        raise SystemExit("M9 LibIPC: signature d'attente synchrone introuvable")
    sonde = anchor + (
        "    if (getenv(\"BOUCHAUD_M9\") != nullptr)\n"
        "        outln(\"[ladybird-bouchaud] M9_IPC_SYNC_WAIT endpoint={} message={}\", endpoint_magic, message_id);\n")
    data = data.replace(anchor, sonde, 1)
    if "<cstdlib>" not in data:
        first_include = data.index("#include ")
        data = data[:first_include] + "#include <cstdlib>\n#include <AK/Format.h>\n" + data[first_include:]
    ipc_cpp.write_text(data)


# PageClient: local policy + M9 screenshot bridge.
page_cpp = root / "Services/WebContent/PageClient.cpp"
data = page_cpp.read_text()

# `bouchaud_m9_expected_url()` appelle `URL::create_with_url_or_path`, declaree
# dans <LibURL/URL.h>. PageClient.cpp manipule des `URL::URL` et obtient donc
# l'en-tete par inclusion transitive — mais s'appuyer la-dessus, c'est accepter
# qu'un remaniement d'upstream casse notre compilation quarante minutes apres le
# debut du build. On l'inclut explicitement.
if "#include <LibURL/URL.h>" not in data:
    url_anchor = "#include <LibWeb/CSS/CSSImportRule.h>\n"
    if url_anchor not in data:
        raise SystemExit("M9 PageClient : ancre d'inclusion LibWeb introuvable")
    data = data.replace(url_anchor, "#include <LibURL/URL.h>\n" + url_anchor, 1)

if "static bool bouchaud_m9_enabled()" not in data:
    old = '''static bool bouchaud_m8_enabled()
{
    return getenv("BOUCHAUD_M8") != nullptr;
}
'''
    new = '''static bool bouchaud_m8_enabled()
{
    return getenv("BOUCHAUD_M8") != nullptr;
}

static bool bouchaud_m9_enabled()
{
    return getenv("BOUCHAUD_M9") != nullptr;
}

// L'URL que M9 attend, sous sa forme serialisee — la meme source et la meme
// valeur par defaut que le bootstrap de ConnectionFromClient, pour que les deux
// cotes ne puissent pas diverger.
static ByteString bouchaud_m9_expected_url()
{
    char const* requested = getenv("BOUCHAUD_M9_URL");
    if (!requested || !*requested)
        requested = "http://10.0.2.2:18080/m9.html";
    auto url = URL::create_with_url_or_path(ByteString { requested });
    if (!url.has_value())
        return ByteString { requested };
    return url->to_byte_string();
}
'''
    if old not in data:
        raise SystemExit("M9 PageClient M8 enable helper missing")
    data = data.replace(old, new, 1)

# Duplicate the validated M8 presentation bridge mechanically. M8 remains
# byte-for-byte unchanged; only the duplicated function uses M9 markers/title.
if "static bool bouchaud_m9_present(" not in data:
    start = data.find("static bool bouchaud_m8_present(")
    if start < 0:
        raise SystemExit("M9 cannot find M8 presentation bridge")
    endif = data.find("\n#endif", start)
    if endif < 0:
        raise SystemExit("M9 cannot find end of M8 presentation bridge")

    m8_function = data[start:endif]
    m9_function = (
        m8_function
        .replace("bouchaud_m8_present", "bouchaud_m9_present")
        .replace("M8_", "M9_")
        .replace("Ladybird M8 - HTML local", "Ladybird M9 - HTTP distant")
    )
    data = data[:endif] + "\n\n" + m9_function + data[endif:]

old_allocate = '''#if defined(BOUCHAUD_PORT)
    if (bouchaud_m8_enabled())
        return Web::Compositor::compositor_context_id_for_page(m_id);
#endif'''
new_allocate = '''#if defined(BOUCHAUD_PORT)
    if (bouchaud_m8_enabled() || bouchaud_m9_enabled())
        return Web::Compositor::compositor_context_id_for_page(m_id);
#endif'''
if old_allocate in data:
    data = data.replace(old_allocate, new_allocate, 1)
elif new_allocate not in data:
    raise SystemExit("M9 compositor allocation hook missing")

old_decision = '''Web::NavigationProcessDecision PageClient::decide_navigation_process(URL::URL const& current_url, URL::URL const& target_url, Web::NavigationTarget target, Optional<Web::HTML::CrossProcessId> frame_id) const
{
    return client().decide_navigation_process(m_id, move(frame_id), current_url, target_url, target);
}'''
new_decision = '''Web::NavigationProcessDecision PageClient::decide_navigation_process(URL::URL const& current_url, URL::URL const& target_url, Web::NavigationTarget target, Optional<Web::HTML::CrossProcessId> frame_id) const
{
#if defined(BOUCHAUD_PORT)
    if (bouchaud_m9_enabled())
        return Web::NavigationProcessDecision::Local;
#endif
    return client().decide_navigation_process(m_id, move(frame_id), current_url, target_url, target);
}'''
if old_decision in data:
    data = data.replace(old_decision, new_decision, 1)
elif new_decision not in data:
    raise SystemExit("M9 navigation process decision hook missing")

# Sonde d'annulation. Ancree sur la signature seule : le corps d'upstream porte
# aussi une branche WebDriver, et recopier un corps entier fait dependre notre
# patch de details qui ne nous concernent pas.
old_cancel = """void PageClient::page_did_cancel_loading(Optional<Utf16String> const& navigation_id, URL::URL const& url)
{
"""
new_cancel = """void PageClient::page_did_cancel_loading(Optional<Utf16String> const& navigation_id, URL::URL const& url)
{
#if defined(BOUCHAUD_PORT)
    // Sans cette sonde, un chargement abandonne est indiscernable d'un
    // chargement qui n'arrive jamais : dans les deux cas le journal se tait, et
    // l'on ne sait pas s'il faut chercher dans le reseau ou dans la navigation.
    if (bouchaud_m9_enabled())
        outln("[ladybird-bouchaud] M9_NAVIGATION_CANCELLED page={} url={}", m_id, url);
#endif
"""
if new_cancel in data:
    pass
elif old_cancel in data:
    data = data.replace(old_cancel, new_cancel, 1)
else:
    raise SystemExit("M9 cancel-loading probe missing")

old_change_url = '''void PageClient::page_did_change_url(URL::URL const& url)
{
    client().async_did_change_url(m_id, url);
}'''
new_change_url = '''void PageClient::page_did_change_url(URL::URL const& url)
{
#if defined(BOUCHAUD_PORT)
    if (bouchaud_m9_enabled()) {
        outln("[ladybird-bouchaud] M9_NAVIGATION_COMMITTED page={} url={}", m_id, url);
        return;
    }
#endif
    client().async_did_change_url(m_id, url);
}'''
if old_change_url in data:
    data = data.replace(old_change_url, new_change_url, 1)
elif new_change_url not in data:
    raise SystemExit("M9 change-url hook missing")

old_start_loading = '''void PageClient::page_did_start_loading(Optional<Utf16String> const& navigation_id, URL::URL const& url, Web::HTML::DocumentResource document_resource, bool is_redirect, Web::Bindings::NavigationHistoryBehavior history_handling)
{
    if (m_webdriver)
        m_webdriver->page_did_start_loading({}, url);

    client().async_did_start_loading(m_id, navigation_id, url, move(document_resource), is_redirect, history_handling);
}'''
new_start_loading = '''void PageClient::page_did_start_loading(Optional<Utf16String> const& navigation_id, URL::URL const& url, Web::HTML::DocumentResource document_resource, bool is_redirect, Web::Bindings::NavigationHistoryBehavior history_handling)
{
    if (m_webdriver)
        m_webdriver->page_did_start_loading({}, url);

#if defined(BOUCHAUD_PORT)
    if (bouchaud_m9_enabled()) {
        outln("[ladybird-bouchaud] M9_NAVIGATION_STARTED page={} url={} redirect={}", m_id, url, is_redirect);
        return;
    }
#endif
    client().async_did_start_loading(m_id, navigation_id, url, move(document_resource), is_redirect, history_handling);
}'''
if old_start_loading in data:
    data = data.replace(old_start_loading, new_start_loading, 1)
elif new_start_loading not in data:
    raise SystemExit("M9 start-loading hook missing")

old_finish = '''void PageClient::page_did_finish_loading(Optional<Utf16String> const& navigation_id, URL::URL const& url)
{
#if defined(BOUCHAUD_PORT)
    if (bouchaud_m8_enabled()) {
        outln("[ladybird-bouchaud] M8_LOCAL_HTML_RENDERED page={} url={}", m_id, url);
        page().top_level_traversable()->queue_screenshot_task({});
        return;
    }
#endif
    client().async_did_finish_loading(m_id, navigation_id, url);
}'''
new_finish = '''void PageClient::page_did_finish_loading(Optional<Utf16String> const& navigation_id, URL::URL const& url)
{
#if defined(BOUCHAUD_PORT)
    if (bouchaud_m8_enabled()) {
        outln("[ladybird-bouchaud] M8_LOCAL_HTML_RENDERED page={} url={}", m_id, url);
        page().top_level_traversable()->queue_screenshot_task({});
        return;
    }
    if (bouchaud_m9_enabled()) {
        // `page_did_finish_loading` se declenche pour **chaque** document, y
        // compris le `about:blank` initial que Ladybird termine pendant que la
        // navigation HTTP est encore en vol. Capturer la sans distinction
        // photographiait une page blanche et faisait echouer l'invariant de
        // contenu — le defaut n'etait pas le rendu, mais le moment.
        auto chargee = url.to_byte_string();
        auto attendue = bouchaud_m9_expected_url();
        if (chargee != attendue) {
            outln("[ladybird-bouchaud] M9_DOCUMENT_SKIPPED page={} url={} attendu={}", m_id, chargee, attendue);
            return;
        }
        outln("[ladybird-bouchaud] M9_DOCUMENT_LOADED page={} url={}", m_id, chargee);
        outln("[ladybird-bouchaud] M9_CAPTURE_MATCH url={}", chargee);
        page().top_level_traversable()->queue_screenshot_task({});
        return;
    }
#endif
    client().async_did_finish_loading(m_id, navigation_id, url);
}'''
if old_finish in data:
    data = data.replace(old_finish, new_finish, 1)
elif new_finish not in data:
    raise SystemExit("M9 finish-loading hook missing")

old_screenshot = '''void PageClient::page_did_take_screenshot(Gfx::ShareableBitmap const& screenshot)
{
#if defined(BOUCHAUD_PORT)
    if (bouchaud_m8_enabled()) {
        Core::Process::terminate_immediately(bouchaud_m8_present(screenshot) ? 0 : 70);
    }
#endif
    client().async_did_take_screenshot(m_id, screenshot);
}'''
new_screenshot = '''void PageClient::page_did_take_screenshot(Gfx::ShareableBitmap const& screenshot)
{
#if defined(BOUCHAUD_PORT)
    if (bouchaud_m8_enabled()) {
        Core::Process::terminate_immediately(bouchaud_m8_present(screenshot) ? 0 : 70);
    }
    if (bouchaud_m9_enabled()) {
        if (!bouchaud_m9_present(screenshot))
            Core::Process::terminate_immediately(70);

        outln("[ladybird-bouchaud] M9_CPU_SCREENSHOT_RENDERED");

        if (getenv("BOUCHAUD_M9_TEST"))
            Core::Process::terminate_immediately(0);

        outln("[ladybird-bouchaud] M9_WEBCONTENT_STILL_ALIVE");
        return;
    }
#endif
    client().async_did_take_screenshot(m_id, screenshot);
}'''
if old_screenshot in data:
    data = data.replace(old_screenshot, new_screenshot, 1)
elif new_screenshot not in data:
    raise SystemExit("M9 screenshot hook missing")

old_cookie = '''HTTP::Cookie::VersionedCookie PageClient::page_did_request_cookie(URL::URL const& url, HTTP::Cookie::Source source)
{
    auto response = client().send_sync_but_allow_failure<Messages::WebContentClient::DidRequestCookie>(m_id, url, source);'''
new_cookie = '''HTTP::Cookie::VersionedCookie PageClient::page_did_request_cookie(URL::URL const& url, HTTP::Cookie::Source source)
{
#if defined(BOUCHAUD_PORT)
    if (bouchaud_m9_enabled())
        return {};
#endif
    auto response = client().send_sync_but_allow_failure<Messages::WebContentClient::DidRequestCookie>(m_id, url, source);'''
if old_cookie in data:
    data = data.replace(old_cookie, new_cookie, 1)
elif new_cookie not in data:
    raise SystemExit("M9 cookie request hook missing")

old_set_cookie = '''void PageClient::page_did_set_cookie(URL::URL const& url, HTTP::Cookie::ParsedCookie const& cookie, HTTP::Cookie::Source source)
{
    auto response = client().send_sync_but_allow_failure<Messages::WebContentClient::DidSetCookie>(url, cookie, source);'''
new_set_cookie = '''void PageClient::page_did_set_cookie(URL::URL const& url, HTTP::Cookie::ParsedCookie const& cookie, HTTP::Cookie::Source source)
{
#if defined(BOUCHAUD_PORT)
    if (bouchaud_m9_enabled())
        return;
#endif
    auto response = client().send_sync_but_allow_failure<Messages::WebContentClient::DidSetCookie>(url, cookie, source);'''
if old_set_cookie in data:
    data = data.replace(old_set_cookie, new_set_cookie, 1)
elif new_set_cookie not in data:
    raise SystemExit("M9 cookie set hook missing")

old_hsts = '''bool PageClient::page_did_is_known_hsts_host(String const& domain)
{
    auto response = client().send_sync_but_allow_failure<Messages::WebContentClient::DidIsKnownHstsHost>(domain);'''
new_hsts = '''bool PageClient::page_did_is_known_hsts_host(String const& domain)
{
#if defined(BOUCHAUD_PORT)
    if (bouchaud_m9_enabled())
        return false;
#endif
    auto response = client().send_sync_but_allow_failure<Messages::WebContentClient::DidIsKnownHstsHost>(domain);'''
if old_hsts in data:
    data = data.replace(old_hsts, new_hsts, 1)
elif new_hsts not in data:
    raise SystemExit("M9 HSTS hook missing")

page_cpp.write_text(data)

print("Bouchaud M9 RequestServer + HTTP navigation adaptations applied to", root)