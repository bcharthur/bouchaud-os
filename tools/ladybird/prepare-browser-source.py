#!/usr/bin/env python3
"""Prepare a disposable Ladybird worktree for the Bouchaud service build.

The pinned upstream tree remains untouched. This script only edits the disposable
worktree passed as argv[1]. Changes are deliberately tiny and mechanical:
- configure Services without adding the desktop UI;
- select the unimplemented renderer sandbox for Bouchaud (M14 comes later);
- provide a deterministic x86_64 cache-line alignment when building AK with Clang;
- force a portable x86-64 ISA instead of the CI host's -march=native;
- allow WebContent to inherit an already-created IPC fd from Bouchaud;
- for M8 only, bootstrap one local HTML page inside WebContent, render it with
  LibWeb/LibGfx, copy the resulting CPU bitmap into the Bouchaud shared window
  surface, then announce FrameReady to the Bouchaud window manager.
"""
from pathlib import Path
import sys

if len(sys.argv) != 2:
    raise SystemExit("usage: prepare-browser-source.py <ladybird-worktree>")
root = Path(sys.argv[1])


def replace_once(path: Path, old: str, new: str):
    data = path.read_text()
    if new in data:
        return
    if old not in data:
        raise SystemExit(f"pattern not found in {path}")
    path.write_text(data.replace(old, new, 1))


# Root: Services yes, Qt/AppKit UI no.
cmake = root / "CMakeLists.txt"
replace_once(
    cmake,
    "if (ENABLE_GUI_TARGETS)\n    add_subdirectory(Services)\n    add_subdirectory(UI)\nendif()",
    "if (ENABLE_GUI_TARGETS)\n    add_subdirectory(Services)\n    if (NOT BOUCHAUD_SERVICES_ONLY)\n        add_subdirectory(UI)\n    endif()\nendif()",
)
# Export a compile-time marker to the tiny WebContent bootstrap adaptation.
data = cmake.read_text()
needle = 'include(lagom_install)\n'
insert = 'if (BOUCHAUD_PORT)\n    add_compile_definitions(BOUCHAUD_PORT=1)\nendif()\n\n'
if insert not in data:
    if needle not in data:
        raise SystemExit("cannot place BOUCHAUD_PORT marker")
    cmake.write_text(data.replace(needle, insert + needle, 1))

# Ladybird normally compiles a non-cross host build with -march=native. On a
# GitHub runner this enabled AVX and the resulting WebContent immediately hit
# #UD in Bouchaud/QEMU on `vxorps` before WEBCONTENT_READY. Bouchaud's x86-64
# ABI deliberately targets the architectural baseline (SSE2, no AVX required),
# so override upstream's host tuning only in the disposable BOUCHAUD_PORT tree.
# This must be global: WebContent links hundreds of static Ladybird libraries,
# and one AVX instruction in any of them is enough to crash at startup.
compile_options = root / "Meta/CMake/compile_options.cmake"
replace_once(
    compile_options,
    "if (ENABLE_CI_BASELINE_CPU)\n",
    "if (BOUCHAUD_PORT AND CMAKE_SYSTEM_PROCESSOR MATCHES \"^(x86_64|amd64|AMD64)$\")\n"
    "    add_cxx_compile_options(-march=x86-64 -mtune=generic)\n"
    "elseif (ENABLE_CI_BASELINE_CPU)\n",
)

# AK cache alignment: upstream currently falls back unconditionally to
# __GCC_DESTRUCTIVE_SIZE. GCC exposes that implementation macro, but the Clang
# toolchain deliberately used for Ladybird does not, so every user of
# AK_CACHE_ALIGNED fails to compile. Bouchaud is x86_64-only today; use the
# conventional 64-byte cache-line alignment for this disposable port worktree.
# Do not fake a GCC builtin globally and do not modify the pinned upstream tree.
platform = root / "AK/Platform.h"
replace_once(
    platform,
    "#ifndef AK_SYSTEM_CACHE_ALIGNMENT_SIZE\n#    define AK_SYSTEM_CACHE_ALIGNMENT_SIZE __GCC_DESTRUCTIVE_SIZE\n#endif",
    "#ifndef AK_SYSTEM_CACHE_ALIGNMENT_SIZE\n#    if defined(BOUCHAUD_PORT)\n#        define AK_SYSTEM_CACHE_ALIGNMENT_SIZE 64\n#    else\n#        define AK_SYSTEM_CACHE_ALIGNMENT_SIZE __GCC_DESTRUCTIVE_SIZE\n#    endif\n#endif",
)

# Sandbox: Bouchaud deliberately uses upstream's no-op implementation at M7/M8.
svc = root / "Services/WebContent/CMakeLists.txt"
replace_once(
    svc,
    "if (LINUX)\n    target_sources(WebContent PRIVATE ../RendererSandboxLinux.cpp)",
    "if (BOUCHAUD_PORT)\n    target_sources(WebContent PRIVATE ../RendererSandboxUnimplemented.cpp)\nelseif (LINUX)\n    target_sources(WebContent PRIVATE ../RendererSandboxLinux.cpp)",
)

# WebContent: when the Bouchaud launcher exports BOUCHAUD_WEBCONTENT_FD, adopt
# that socket directly instead of asking Ladybird's desktop SystemServer.
main = root / "Services/WebContent/main.cpp"
data = main.read_text()
if "BOUCHAUD_WEBCONTENT_FD" not in data:
    data = data.replace(
        "#include <LibCore/LocalServer.h>\n",
        "#include <LibCore/LocalServer.h>\n#include <LibCore/Socket.h>\n",
        1,
    )
    if "#include <cstdlib>\n" not in data:
        data = data.replace("#include <SDL3/SDL_init.h>\n", "#include <SDL3/SDL_init.h>\n#include <cstdlib>\n", 1)
    old = '''#if defined(AK_OS_MACOS)\n    auto browser_port = TRY(Core::MachPort::look_up_from_bootstrap_server(ByteString { mach_server_name }));\n    auto transport_ports = TRY(IPC::bootstrap_transport_from_server_port(browser_port));\n    auto webcontent_client = WebContent::ConnectionFromClient::construct(\n        make<IPC::Transport>(move(transport_ports.receive_right), move(transport_ports.send_right)));\n#else\n    auto webcontent_client = TRY(IPC::take_over_accepted_client_from_system_server<WebContent::ConnectionFromClient>(mach_server_name));\n#endif'''
    new = '''#if defined(AK_OS_MACOS)\n    auto browser_port = TRY(Core::MachPort::look_up_from_bootstrap_server(ByteString { mach_server_name }));\n    auto transport_ports = TRY(IPC::bootstrap_transport_from_server_port(browser_port));\n    auto webcontent_client = WebContent::ConnectionFromClient::construct(\n        make<IPC::Transport>(move(transport_ports.receive_right), move(transport_ports.send_right)));\n#elif defined(BOUCHAUD_PORT)\n    auto* inherited_fd = getenv("BOUCHAUD_WEBCONTENT_FD");\n    if (!inherited_fd) {\n        warnln("Bouchaud: BOUCHAUD_WEBCONTENT_FD absent");\n        return 64;\n    }\n    auto fd = atoi(inherited_fd);\n    if (fd < 0) {\n        warnln("Bouchaud: descripteur IPC invalide");\n        return 64;\n    }\n    auto socket = TRY(Core::LocalSocket::adopt_fd(fd));\n    auto webcontent_client = WebContent::ConnectionFromClient::construct(make<IPC::Transport>(move(socket)));\n    outln("[ladybird-bouchaud] WEBCONTENT_READY pid={} fd={}", Core::System::getpid(), fd);\n#else\n    auto webcontent_client = TRY(IPC::take_over_accepted_client_from_system_server<WebContent::ConnectionFromClient>(mach_server_name));\n#endif'''
    if old not in data:
        raise SystemExit("WebContent bootstrap block changed upstream")
    data = data.replace(old, new, 1)
    old_callbacks = '''    webcontent_client->on_image_decoder_connection = [](auto const& handle) {\n        if (auto result = connect_to_image_decoder(handle); result.is_error())\n            dbgln("Failed to connect to image decoder: {}", result.error());\n    };\n\n    return event_loop.exec();'''
    new_callbacks = '''    webcontent_client->on_image_decoder_connection = [](auto const& handle) {\n        if (auto result = connect_to_image_decoder(handle); result.is_error())\n            dbgln("Failed to connect to image decoder: {}", result.error());\n    };\n\n#if defined(BOUCHAUD_PORT)\n    if (getenv("BOUCHAUD_M8"))\n        webcontent_client->bouchaud_m8_start();\n#endif\n\n    return event_loop.exec();'''
    if old_callbacks not in data:
        raise SystemExit("WebContent callback block changed upstream")
    main.write_text(data.replace(old_callbacks, new_callbacks, 1))

# M8 owns the UI side of the tiny local-page bootstrap inside WebContent. This
# does not replace Ladybird's normal IPC protocol: it is enabled only by the
# BOUCHAUD_M8 environment variable used by the QEMU milestone scenario.
connection_h = root / "Services/WebContent/ConnectionFromClient.h"
replace_once(
    connection_h,
    "    void update_input_method_state(u64 page_id);\n\nprivate:",
    "    void update_input_method_state(u64 page_id);\n\n#if defined(BOUCHAUD_PORT)\n    void bouchaud_m8_start();\n#endif\n\nprivate:",
)

connection_cpp = root / "Services/WebContent/ConnectionFromClient.cpp"
data = connection_cpp.read_text()
if "void ConnectionFromClient::bouchaud_m8_start()" not in data:
    if "#include <LibGfx/Size.h>\n" not in data:
        data = data.replace("#include <LibGfx/Color.h>\n", "#include <LibGfx/Color.h>\n#include <LibGfx/Size.h>\n", 1)
    if "#include <cstdlib>\n" not in data:
        data = data.replace("#include <AK/Utf16String.h>\n", "#include <AK/Utf16String.h>\n#include <cstdlib>\n", 1)
    marker = "ConnectionFromClient::~ConnectionFromClient() = default;\n"
    addition = r'''

#if defined(BOUCHAUD_PORT)
static int bouchaud_env_positive_int(char const* name, int fallback)
{
    auto* value = getenv(name);
    if (!value)
        return fallback;
    auto parsed = atoi(value);
    return parsed > 0 ? parsed : fallback;
}

void ConnectionFromClient::bouchaud_m8_start()
{
    constexpr u64 page_id = 1;
    auto width = bouchaud_env_positive_int("BO_SURFACE_WIDTH", 1100);
    auto height = bouchaud_env_positive_int("BO_SURFACE_HEIGHT", 604);

    Web::HTML::CrossProcessId root_navigable_id { .namespace_id = 1, .local_id = 1 };
    Web::HTML::CrossProcessIdAllocator allocator { .namespace_id = 1, .next_local_id = 2 };
    initialize(page_id, root_navigable_id, allocator);

    auto viewport = Gfx::IntSize { width, height }.to_type<Web::DevicePixels>();
    auto screen = Gfx::IntRect { 0, 0, width, height }.to_type<Web::DevicePixels>();
    update_screen_rects(page_id, Vector<Web::DevicePixelRect> { screen }, 0);
    set_viewport(page_id, viewport, 1.0, Web::ViewportIsFullscreen::No);
    set_window_size(page_id, viewport);
    set_has_focus(page_id, true);
    set_system_visibility_state(page_id, Web::HTML::VisibilityState::Visible);

    static constexpr auto html = "<!doctype html><html><head><title>Bouchaud M8</title></head><body><h1>Ladybird sur Bouchaud OS</h1><p>HTML local rendu par LibWeb.</p><p><strong>M8</strong> : WebContent vers fenetre Bouchaud.</p></body></html>";
    outln("[ladybird-bouchaud] M8_BOOTSTRAP page={} viewport={}x{}", page_id, width, height);
    load_html(page_id, ByteString { html });
}
#endif
'''
    if marker not in data:
        raise SystemExit("ConnectionFromClient destructor marker changed upstream")
    connection_cpp.write_text(data.replace(marker, marker + addition, 1))

# M8 screenshot bridge. Ladybird already knows how to render a complete document
# into a CPU BGRA bitmap. We normalize those pixels to Bouchaud XRGB8888, copy
# them into the shared surface supplied by the WM, compare hashes after the copy,
# and then emit the existing Bouchaud GUI protocol (Hello, title, FrameReady).
page_cpp = root / "Services/WebContent/PageClient.cpp"
data = page_cpp.read_text()
if "M8_CAPTURE_MATCH" not in data:
    include_anchor = "#include <WebContent/WebUIConnection.h>\n"
    includes = '''#include <WebContent/WebUIConnection.h>\n\n#if defined(BOUCHAUD_PORT)\n#    include <cerrno>\n#    include <cstdint>\n#    include <cstdlib>\n#    include <cstring>\n#    include <sys/mman.h>\n#    include <unistd.h>\n#endif\n'''
    if include_anchor not in data:
        raise SystemExit("PageClient include anchor changed upstream")
    data = data.replace(include_anchor, includes, 1)

    namespace_anchor = "namespace WebContent {\n\n"
    helpers = r'''namespace WebContent {

#if defined(BOUCHAUD_PORT)
static bool bouchaud_m8_enabled()
{
    return getenv("BOUCHAUD_M8") != nullptr;
}

static int bouchaud_env_fd(char const* name)
{
    auto* value = getenv(name);
    return value ? atoi(value) : -1;
}

static int bouchaud_env_dimension(char const* name, int fallback)
{
    auto* value = getenv(name);
    if (!value)
        return fallback;
    auto parsed = atoi(value);
    return parsed > 0 ? parsed : fallback;
}

static bool bouchaud_write_all(int fd, void const* data, size_t size)
{
    auto* bytes = static_cast<u8 const*>(data);
    while (size > 0) {
        auto written = write(fd, bytes, size);
        if (written < 0 && errno == EINTR)
            continue;
        if (written <= 0)
            return false;
        bytes += written;
        size -= static_cast<size_t>(written);
    }
    return true;
}

static bool bouchaud_gui_message(int fd, u16 kind, u32 serial, void const* payload, u32 payload_size)
{
    u8 header[16] {};
    auto put16 = [](u8* out, u16 value) {
        out[0] = static_cast<u8>(value);
        out[1] = static_cast<u8>(value >> 8);
    };
    auto put32 = [](u8* out, u32 value) {
        out[0] = static_cast<u8>(value);
        out[1] = static_cast<u8>(value >> 8);
        out[2] = static_cast<u8>(value >> 16);
        out[3] = static_cast<u8>(value >> 24);
    };
    put32(header + 0, 0x55474f42); // "BOGU" little-endian
    put16(header + 4, 1);          // protocol version
    put16(header + 6, kind);
    put32(header + 8, payload_size);
    put32(header + 12, serial);
    if (!bouchaud_write_all(fd, header, sizeof(header)))
        return false;
    return payload_size == 0 || bouchaud_write_all(fd, payload, payload_size);
}

static u64 bouchaud_fnv1a_u32(u64 hash, u32 value)
{
    constexpr u64 prime = 1099511628211ULL;
    for (unsigned shift = 0; shift < 32; shift += 8) {
        hash ^= static_cast<u8>(value >> shift);
        hash *= prime;
    }
    return hash;
}

static bool bouchaud_m8_present(Gfx::ShareableBitmap const& screenshot)
{
    if (!screenshot.is_valid() || !screenshot.bitmap()) {
        warnln("[ladybird-bouchaud] M8_CAPTURE_INVALID");
        return false;
    }

    auto const& bitmap = *screenshot.bitmap();
    auto surface_fd = bouchaud_env_fd("BO_SURFACE_FD");
    auto gui_fd = bouchaud_env_fd("BO_GUI_FD");
    auto surface_width = bouchaud_env_dimension("BO_SURFACE_WIDTH", bitmap.width());
    auto surface_height = bouchaud_env_dimension("BO_SURFACE_HEIGHT", bitmap.height());
    auto stride = bouchaud_env_dimension("BO_SURFACE_STRIDE", surface_width * 4);
    if (surface_fd < 0 || gui_fd < 0 || surface_width <= 0 || surface_height <= 0 || stride < surface_width * 4) {
        warnln("[ladybird-bouchaud] M8_SURFACE_ENV_INVALID surface_fd={} gui_fd={} {}x{} stride={}", surface_fd, gui_fd, surface_width, surface_height, stride);
        return false;
    }

    auto surface_bytes = static_cast<size_t>(stride) * static_cast<size_t>(surface_height);
    auto* mapped = mmap(nullptr, surface_bytes, PROT_READ | PROT_WRITE, MAP_SHARED, surface_fd, 0);
    if (mapped == MAP_FAILED) {
        warnln("[ladybird-bouchaud] M8_SURFACE_MMAP_FAILED errno={}", errno);
        return false;
    }

    auto* destination = static_cast<u8*>(mapped);
    for (int y = 0; y < surface_height; ++y) {
        auto* row = reinterpret_cast<u32*>(destination + static_cast<size_t>(y) * stride);
        for (int x = 0; x < surface_width; ++x)
            row[x] = 0x00ffffff;
    }

    auto copy_width = min(bitmap.width(), surface_width);
    auto copy_height = min(bitmap.height(), surface_height);
    constexpr u64 fnv_offset = 1469598103934665603ULL;
    auto source_hash = fnv_offset;
    for (int y = 0; y < copy_height; ++y) {
        auto const* source = bitmap.scanline(y);
        auto* row = reinterpret_cast<u32*>(destination + static_cast<size_t>(y) * stride);
        for (int x = 0; x < copy_width; ++x) {
            auto pixel = source[x] & 0x00ffffffu;
            row[x] = pixel;
            source_hash = bouchaud_fnv1a_u32(source_hash, pixel);
        }
    }

    auto destination_hash = fnv_offset;
    for (int y = 0; y < copy_height; ++y) {
        auto const* row = reinterpret_cast<u32 const*>(destination + static_cast<size_t>(y) * stride);
        for (int x = 0; x < copy_width; ++x)
            destination_hash = bouchaud_fnv1a_u32(destination_hash, row[x]);
    }

    if (source_hash != destination_hash) {
        warnln("[ladybird-bouchaud] M8_CAPTURE_MISMATCH source={:#x} destination={:#x}", source_hash, destination_hash);
        munmap(mapped, surface_bytes);
        return false;
    }

    static constexpr char title[] = "Ladybird M8 - HTML local";
    if (!bouchaud_gui_message(gui_fd, 1, 1, nullptr, 0)
        || !bouchaud_gui_message(gui_fd, 3, 2, title, sizeof(title) - 1)) {
        warnln("[ladybird-bouchaud] M8_GUI_HANDSHAKE_FAILED errno={}", errno);
        munmap(mapped, surface_bytes);
        return false;
    }

    u8 frame[24] {};
    auto put32 = [](u8* out, u32 value) {
        out[0] = static_cast<u8>(value);
        out[1] = static_cast<u8>(value >> 8);
        out[2] = static_cast<u8>(value >> 16);
        out[3] = static_cast<u8>(value >> 24);
    };
    put32(frame + 0, 1); // window id
    put32(frame + 4, 0); // buffer id
    put32(frame + 8, 0); // damage x
    put32(frame + 12, 0); // damage y
    put32(frame + 16, static_cast<u32>(surface_width));
    put32(frame + 20, static_cast<u32>(surface_height));
    auto frame_ok = bouchaud_gui_message(gui_fd, 6, 3, frame, sizeof(frame));
    munmap(mapped, surface_bytes);
    if (!frame_ok) {
        warnln("[ladybird-bouchaud] M8_FRAME_READY_FAILED errno={}", errno);
        return false;
    }

    outln("[ladybird-bouchaud] M8_CAPTURE_MATCH hash={:#x} bitmap={}x{} window={}x{}", source_hash, bitmap.width(), bitmap.height(), surface_width, surface_height);
    return true;
}
#endif

'''
    if namespace_anchor not in data:
        raise SystemExit("PageClient namespace marker changed upstream")
    data = data.replace(namespace_anchor, helpers, 1)

    old_allocate = '''Web::Compositor::CompositorContextId PageClient::allocate_compositor_context_id(Web::Compositor::PagePresentationRegistration page_presentation_registration)\n{\n    return client().allocate_compositor_context_id(m_id, page_presentation_registration);\n}'''
    new_allocate = '''Web::Compositor::CompositorContextId PageClient::allocate_compositor_context_id(Web::Compositor::PagePresentationRegistration page_presentation_registration)\n{\n#if defined(BOUCHAUD_PORT)\n    if (bouchaud_m8_enabled())\n        return Web::Compositor::compositor_context_id_for_page(m_id);\n#endif\n    return client().allocate_compositor_context_id(m_id, page_presentation_registration);\n}'''
    if old_allocate not in data:
        raise SystemExit("PageClient compositor allocation changed upstream")
    data = data.replace(old_allocate, new_allocate, 1)

    old_finish = '''void PageClient::page_did_finish_loading(Optional<Utf16String> const& navigation_id, URL::URL const& url)\n{\n    client().async_did_finish_loading(m_id, navigation_id, url);\n}'''
    new_finish = '''void PageClient::page_did_finish_loading(Optional<Utf16String> const& navigation_id, URL::URL const& url)\n{\n#if defined(BOUCHAUD_PORT)\n    if (bouchaud_m8_enabled()) {\n        outln("[ladybird-bouchaud] M8_LOCAL_HTML_RENDERED page={} url={}", m_id, url);\n        page().top_level_traversable()->queue_screenshot_task({});\n        return;\n    }\n#endif\n    client().async_did_finish_loading(m_id, navigation_id, url);\n}'''
    if old_finish not in data:
        raise SystemExit("PageClient finish-loading hook changed upstream")
    data = data.replace(old_finish, new_finish, 1)

    old_screenshot = '''void PageClient::page_did_take_screenshot(Gfx::ShareableBitmap const& screenshot)\n{\n    client().async_did_take_screenshot(m_id, screenshot);\n}'''
    new_screenshot = '''void PageClient::page_did_take_screenshot(Gfx::ShareableBitmap const& screenshot)\n{\n#if defined(BOUCHAUD_PORT)\n    if (bouchaud_m8_enabled()) {\n        Core::Process::terminate_immediately(bouchaud_m8_present(screenshot) ? 0 : 70);\n    }\n#endif\n    client().async_did_take_screenshot(m_id, screenshot);\n}'''
    if old_screenshot not in data:
        raise SystemExit("PageClient screenshot hook changed upstream")
    page_cpp.write_text(data.replace(old_screenshot, new_screenshot, 1))

print("Bouchaud browser adaptations applied to", root)
