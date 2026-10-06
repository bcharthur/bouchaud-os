#!/usr/bin/env python3
"""Prepare a disposable Ladybird worktree for the Bouchaud service build.

The pinned upstream tree remains untouched. This script only edits the disposable
worktree passed as argv[1]. Changes are deliberately tiny and mechanical:
- configure Services without adding the desktop UI;
- select the Bouchaud renderer sandbox (kernel-applied, process-verified);
- provide a deterministic x86_64 cache-line alignment when building AK with Clang;
- force a portable x86-64 ISA instead of the CI host's -march=native.

WebContent is started by the browser process (UI/Bouchaud) through upstream's
SOCKET_TAKEOVER, exactly like on Linux. The former M8 mode (WebContent adopting
an inherited fd and painting into the window itself) is gone: BOUCHAUD_UI_V1.
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

# Sandbox: BOUCHAUD_SANDBOX_V1 -- the kernel confines WebContent (profile
# BrowserContent) and RendererSandboxBouchaud.cpp verifies it, fail-closed
# (copied by prepare-sandbox-bouchaud.py).
svc = root / "Services/WebContent/CMakeLists.txt"
replace_once(
    svc,
    "if (LINUX)\n    target_sources(WebContent PRIVATE ../RendererSandboxLinux.cpp)",
    "if (BOUCHAUD_PORT)\n    target_sources(WebContent PRIVATE ../RendererSandboxBouchaud.cpp)\nelseif (LINUX)\n    target_sources(WebContent PRIVATE ../RendererSandboxLinux.cpp)",
)

print("Bouchaud browser adaptations applied to", root)
