#!/usr/bin/env python3
"""Prepare a disposable Ladybird worktree for the Bouchaud service build.

The pinned upstream tree remains untouched. This script only edits the disposable
worktree passed as argv[1]. Changes are deliberately tiny and mechanical:
- configure Services without adding the desktop UI;
- select the unimplemented renderer sandbox for Bouchaud (M14 comes later);
- allow WebContent to inherit an already-created IPC fd from Bouchaud.
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
        "#include <LibCore/LocalServer.h>\n#include <LibCore/LocalSocket.h>\n",
        1,
    )
    if "#include <cstdlib>\n" not in data:
        data = data.replace("#include <SDL3/SDL_init.h>\n", "#include <SDL3/SDL_init.h>\n#include <cstdlib>\n", 1)
    old = '''#if defined(AK_OS_MACOS)\n    auto browser_port = TRY(Core::MachPort::look_up_from_bootstrap_server(ByteString { mach_server_name }));\n    auto transport_ports = TRY(IPC::bootstrap_transport_from_server_port(browser_port));\n    auto webcontent_client = WebContent::ConnectionFromClient::construct(\n        make<IPC::Transport>(move(transport_ports.receive_right), move(transport_ports.send_right)));\n#else\n    auto webcontent_client = TRY(IPC::take_over_accepted_client_from_system_server<WebContent::ConnectionFromClient>(mach_server_name));\n#endif'''
    new = '''#if defined(AK_OS_MACOS)\n    auto browser_port = TRY(Core::MachPort::look_up_from_bootstrap_server(ByteString { mach_server_name }));\n    auto transport_ports = TRY(IPC::bootstrap_transport_from_server_port(browser_port));\n    auto webcontent_client = WebContent::ConnectionFromClient::construct(\n        make<IPC::Transport>(move(transport_ports.receive_right), move(transport_ports.send_right)));\n#elif defined(BOUCHAUD_PORT)\n    auto* inherited_fd = getenv("BOUCHAUD_WEBCONTENT_FD");\n    if (!inherited_fd) {\n        warnln("Bouchaud: BOUCHAUD_WEBCONTENT_FD absent");\n        return 64;\n    }\n    auto fd = atoi(inherited_fd);\n    if (fd < 0) {\n        warnln("Bouchaud: descripteur IPC invalide");\n        return 64;\n    }\n    auto socket = TRY(Core::LocalSocket::adopt_fd(fd));\n    auto webcontent_client = WebContent::ConnectionFromClient::construct(make<IPC::Transport>(move(socket)));\n    outln("[ladybird-bouchaud] WEBCONTENT_READY pid={} fd={}", Core::System::getpid(), fd);\n#else\n    auto webcontent_client = TRY(IPC::take_over_accepted_client_from_system_server<WebContent::ConnectionFromClient>(mach_server_name));\n#endif'''
    if old not in data:
        raise SystemExit("WebContent bootstrap block changed upstream")
    main.write_text(data.replace(old, new, 1))

print("Bouchaud browser adaptations applied to", root)
