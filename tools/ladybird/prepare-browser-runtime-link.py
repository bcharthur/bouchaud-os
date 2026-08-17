#!/usr/bin/env python3
"""Bouchaud-only host/target split for the disposable Ladybird worktree.

The native browser build runs on Ubuntu but produces a few runtime executables
for Bouchaud OS. Do not apply Bouchaud's static-PIE/link policy globally:
Ladybird also builds and executes host generators during the build (for example
LibJS/generate_interpreter_layout). Those host tools must remain normal Linux
executables.

This script is intentionally applied only to the disposable Ladybird worktree.
It:
- scopes -static-pie/duplicate-symbol tolerance to Bouchaud runtime services;
- strips build/install RPATH from those static PIE executables: glibc's
  _dl_relocate_static_pie asserts that DT_RPATH/DT_RUNPATH are absent;
- forces the no-op sandbox implementations for Bouchaud services instead of
  selecting the Linux sandbox merely because CMake itself runs on Ubuntu;
- leaves every build-time generator/tool with the native Ubuntu link policy;
- pins LibWebView's resource:// root to /usr/share/ladybird on Bouchaud, matching
  the filesystem layout produced by the QEMU/runtime packaging instead of
  deriving a Linux install prefix from /usr/libexec/ladybird/WebContent;
- disables Ladybird's external Compositor for the M8 CPU-paint path: Bouchaud
  deliberately has no Compositor process at this milestone and presents the
  LibWeb/LibGfx screenshot directly through its own shared window surface;
- restores the normal Browser-side system-font initialization before M8 creates
  its first document, using the SerenitySans resource already embedded by
  WebContent, and verifies that the font really resolved before page creation;
- instruments the M8-only local page bootstrap so a bare-metal failure can be
  located to one initialization stage from the serial log alone;
- parses M8's static HTML directly in LibWeb instead of entering the
  Browser-coordinated session-history navigation path.
"""
from pathlib import Path
import sys

if len(sys.argv) != 2:
    raise SystemExit("usage: prepare-browser-runtime-link.py <ladybird-worktree>")

root = Path(sys.argv[1])


def replace_once(path: Path, old: str, new: str) -> None:
    data = path.read_text()
    if new in data:
        return
    if old not in data:
        raise SystemExit(f"pattern not found in {path}: {old!r}")
    path.write_text(data.replace(old, new, 1))


def append_runtime_link_options(path: Path, target: str) -> None:
    data = path.read_text()
    marker = f"# Bouchaud runtime link policy for {target}"
    if marker in data:
        # Older generated worktrees may already contain the pre-RPATH version.
        # Replace the whole block so rerunning this script is deterministic.
        start = data.index(marker)
        prefix = data[:start].rstrip()
        data = prefix + "\n"
    block = f'''\n{marker}\nif (BOUCHAUD_PORT)\n    target_link_options({target} PRIVATE -static-pie LINKER:--allow-multiple-definition)\n\n    # A glibc static PIE relocates itself before normal libc/TLS startup. Its\n    # elf_get_dynamic_info() path asserts that static PIE binaries do not carry\n    # DT_RPATH or DT_RUNPATH. CMake otherwise injects the vcpkg build directory\n    # as RUNPATH even though every dependency is linked statically. Keep the\n    # link-time search path in LIBRARY_PATH/CMAKE_LIBRARY_PATH, but emit no\n    # runtime search path in the Bouchaud executable.\n    set_target_properties({target} PROPERTIES\n        SKIP_BUILD_RPATH TRUE\n        BUILD_WITH_INSTALL_RPATH FALSE\n        INSTALL_RPATH \"\"\n    )\nendif()\n'''
    path.write_text(data.rstrip() + "\n" + block)


# WebContent already gets its Bouchaud sandbox selection from
# prepare-browser-source.py. Only scope its final executable link policy here.
append_runtime_link_options(root / "Services/WebContent/CMakeLists.txt", "WebContent")

# RequestServer: CMake runs on Linux, but Bouchaud must not compile/run the
# namespace/seccomp Linux sandbox implementation at M7/M8.
request = root / "Services/RequestServer/CMakeLists.txt"
replace_once(
    request,
    "if (LINUX)\n    list(APPEND SOURCES SandboxLinux.cpp)",
    "if (BOUCHAUD_PORT)\n    list(APPEND SOURCES SandboxUnimplemented.cpp)\nelseif (LINUX)\n    list(APPEND SOURCES SandboxLinux.cpp)",
)
append_runtime_link_options(request, "RequestServer")

# ImageDecoder: same host-Linux vs target-Bouchaud distinction.
image = root / "Services/ImageDecoder/CMakeLists.txt"
replace_once(
    image,
    "if (LINUX)\n    target_sources(ImageDecoder PRIVATE SandboxLinux.cpp)",
    "if (BOUCHAUD_PORT)\n    target_sources(ImageDecoder PRIVATE SandboxUnimplemented.cpp)\nelseif (LINUX)\n    target_sources(ImageDecoder PRIVATE SandboxLinux.cpp)",
)
append_runtime_link_options(image, "ImageDecoder")

# WebWorker shares RendererSandbox with WebContent.
worker = root / "Services/WebWorker/CMakeLists.txt"
replace_once(
    worker,
    "if (LINUX)\n    target_sources(WebWorker PRIVATE ../RendererSandboxLinux.cpp)",
    "if (BOUCHAUD_PORT)\n    target_sources(WebWorker PRIVATE ../RendererSandboxUnimplemented.cpp)\nelseif (LINUX)\n    target_sources(WebWorker PRIVATE ../RendererSandboxLinux.cpp)",
)
append_runtime_link_options(worker, "WebWorker")

# Upstream's GPU compositor target is named `Compositor`, not
# `WebContentCompositor`. We prepare it correctly here for a future stage, but
# browser-upstream.sh deliberately does not build it yet: Bouchaud's roadmap is
# CPU Skia -> shared surface -> WM, not ANGLE/OpenGL compositor integration.
compositor = root / "Services/Compositor/CMakeLists.txt"
replace_once(
    compositor,
    "if (LINUX)\n    target_sources(Compositor PRIVATE SandboxLinux.cpp)",
    "if (BOUCHAUD_PORT)\n    target_sources(Compositor PRIVATE SandboxUnimplemented.cpp)\nelseif (LINUX)\n    target_sources(Compositor PRIVATE SandboxLinux.cpp)",
)
append_runtime_link_options(compositor, "Compositor")

# WebView::platform_init() normally derives resource:// from the executable's
# Linux install prefix. Bouchaud intentionally packages helper processes under
# /usr/libexec/ladybird while the milestone disk puts Ladybird resources in
# /usr/share/ladybird. With the upstream derivation that executable location
# becomes /usr/libexec/share/Lagom, so resource://fonts is empty and
# StyleComputer later dereferences a null default-font RefPtr. Make the Bouchaud
# resource root explicit in the disposable target worktree; Linux host tools
# keep the untouched upstream discovery logic.
utilities = root / "Libraries/LibWebView/Utilities.cpp"
replace_once(
    utilities,
    '''    s_ladybird_resource_root = [] {\n        auto home = Core::Environment::get("XDG_CONFIG_HOME"sv)''',
    '''    s_ladybird_resource_root = [] {\n#if defined(BOUCHAUD_PORT)\n        return ByteString { "/usr/share/ladybird" };\n#else\n        auto home = Core::Environment::get("XDG_CONFIG_HOME"sv)''',
)
replace_once(
    utilities,
    '''#endif\n    }();\n\n    Core::ResourceImplementation::install(make<Core::ResourceImplementationFile>(MUST(String::from_byte_string(s_ladybird_resource_root))));''',
    '''#endif\n#endif\n    }();\n\n    Core::ResourceImplementation::install(make<Core::ResourceImplementationFile>(MUST(String::from_byte_string(s_ladybird_resource_root))));''',
)

# M8 intentionally has no Ladybird Compositor process. PageHost::initialize()
# creates the first top-level traversable immediately; with the upstream
# PageClient advertising compositor support, that creation path tries to obtain
# a compositor host from ConnectionFromClient before any compositor connection
# exists and dereferences a null RefPtr. Tell LibWeb to stay on its local CPU
# painting path. M7 never creates a page, so its bootstrap semantics are
# unchanged; a later compositor milestone can remove this Bouchaud override.
page_client_h = root / "Services/WebContent/PageClient.h"
replace_once(
    page_client_h,
    "    virtual bool supports_compositor() const override { return true; }",
    "#if defined(BOUCHAUD_PORT)\n"
    "    virtual bool supports_compositor() const override { return false; }\n"
    "#else\n"
    "    virtual bool supports_compositor() const override { return true; }\n"
    "#endif",
)

# M8 diagnostic stages. Keep these in the disposable source tree: they are
# intentionally verbose only when BOUCHAUD_M8 calls bouchaud_m8_start(). The
# failing run reached WEBCONTENT_READY, printed the old M8_FONT_READY marker,
# then hit RefPtr::as_nonnull_ptr in StyleComputer::StyleComputer(). The marker
# only meant SetSystemFontFamily had been called; it did not prove the family
# existed in FontDatabase. With resource:// fixed above, resolve the exact font
# before page creation and fail explicitly if the packaging ever regresses.
connection = root / "Services/WebContent/ConnectionFromClient.cpp"
replace_once(
    connection,
    "#include <LibWeb/HTML/LocalTraversableNavigable.h>\n",
    "#include <LibWeb/HTML/LocalTraversableNavigable.h>\n#include <LibWeb/HTML/Parser/HTMLParser.h>\n",
)
replace_once(
    connection,
    '''    initialize(page_id, root_navigable_id, allocator);\n\n    auto viewport = Gfx::IntSize { width, height }.to_type<Web::DevicePixels>();''',
    '''    set_system_font_family("SerenitySans"_string);\n    auto m8_default_font = Web::Platform::FontPlugin::the().default_font(16);\n    if (!m8_default_font) {\n        warnln("[ladybird-bouchaud] M8_FONT_MISSING family=SerenitySans resource_root=/usr/share/ladybird");\n        Core::Process::terminate_immediately(71);\n    }\n    outln("[ladybird-bouchaud] M8_FONT_READY family=SerenitySans");\n    outln("[ladybird-bouchaud] M8_STAGE initialize begin");\n    initialize(page_id, root_navigable_id, allocator);\n    outln("[ladybird-bouchaud] M8_STAGE initialize ok");\n\n    auto viewport = Gfx::IntSize { width, height }.to_type<Web::DevicePixels>();''',
)
replace_once(
    connection,
    '''    update_screen_rects(page_id, Vector<Web::DevicePixelRect> { screen }, 0);\n    set_viewport(page_id, viewport, 1.0, Web::ViewportIsFullscreen::No);\n    set_window_size(page_id, viewport);\n    set_has_focus(page_id, true);\n    set_system_visibility_state(page_id, Web::HTML::VisibilityState::Visible);\n\n    static constexpr auto html''',
    '''    outln("[ladybird-bouchaud] M8_STAGE screen begin");\n    update_screen_rects(page_id, Vector<Web::DevicePixelRect> { screen }, 0);\n    outln("[ladybird-bouchaud] M8_STAGE screen ok");\n    set_viewport(page_id, viewport, 1.0, Web::ViewportIsFullscreen::No);\n    outln("[ladybird-bouchaud] M8_STAGE viewport ok");\n    set_window_size(page_id, viewport);\n    outln("[ladybird-bouchaud] M8_STAGE window-size ok");\n    set_has_focus(page_id, true);\n    outln("[ladybird-bouchaud] M8_STAGE focus ok");\n    set_system_visibility_state(page_id, Web::HTML::VisibilityState::Visible);\n    outln("[ladybird-bouchaud] M8_STAGE visibility ok");\n\n    static constexpr auto html''',
)

# Page::load_html() is a real about:srcdoc navigation and asks the Browser
# process to coordinate a session-history operation. M8 deliberately has no
# Browser process yet, so parse the deterministic local document directly with
# LibWeb's real HTMLParser and drive the already-existing screenshot queue from
# WebContent. StringView has no one-argument C-string constructor in the pinned
# AK API; provide the pointer and length explicitly instead of constructing a
# temporary ByteString (the exact compile failure from PR #189).
replace_once(
    connection,
    '''    outln("[ladybird-bouchaud] M8_BOOTSTRAP page={} viewport={}x{}", page_id, width, height);\n    load_html(page_id, ByteString { html });''',
    '''    outln("[ladybird-bouchaud] M8_BOOTSTRAP page={} viewport={}x{}", page_id, width, height);\n\n    auto page_client = this->page(page_id);\n    if (!page_client.has_value()) {\n        warnln("[ladybird-bouchaud] M8_PAGE_MISSING");\n        Core::Process::terminate_immediately(71);\n    }\n\n    auto document = page_client->page().top_level_traversable()->active_document();\n    if (!document) {\n        warnln("[ladybird-bouchaud] M8_DOCUMENT_MISSING");\n        Core::Process::terminate_immediately(72);\n    }\n\n    outln("[ladybird-bouchaud] M8_STAGE parse begin");\n    auto parser = Web::HTML::HTMLParser::create_from_byte_string(\n        *document,\n        StringView { html, __builtin_strlen(html) },\n        Web::HTML::ParserScriptingMode::Disabled,\n        "UTF-8"sv);\n    parser->run(URL::about_srcdoc());\n    outln("[ladybird-bouchaud] M8_STAGE parse ok");\n    outln("[ladybird-bouchaud] M8_LOCAL_HTML_RENDERED page={}", page_id);\n\n    auto traversable = page_client->page().top_level_traversable();\n    traversable->queue_screenshot_task({});\n    traversable->process_screenshot_requests();''',
)

print("Bouchaud host/runtime link split applied to", root)
