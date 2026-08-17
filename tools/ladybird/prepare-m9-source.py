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


# PageClient: local policy + M9 screenshot bridge.
page_cpp = root / "Services/WebContent/PageClient.cpp"
data = page_cpp.read_text()

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
        outln("[ladybird-bouchaud] M9_DOCUMENT_LOADED page={} url={}", m_id, url);
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
