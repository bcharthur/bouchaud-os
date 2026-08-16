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
- forces the no-op sandbox implementations for Bouchaud services instead of
  selecting the Linux sandbox merely because CMake itself runs on Ubuntu;
- leaves every build-time generator/tool with the native Ubuntu link policy.
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
        return
    block = f'''\n{marker}\nif (BOUCHAUD_PORT)\n    target_link_options({target} PRIVATE -static-pie LINKER:--allow-multiple-definition)\nendif()\n'''
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

print("Bouchaud host/runtime link split applied to", root)
