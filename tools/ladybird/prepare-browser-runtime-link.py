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

WebContent keeps upstream's Compositor path (`supports_compositor() == true`):
frames are painted by the Compositor process and presented to the browser
process (UI/Bouchaud). The former M8 CPU-screenshot fallback is gone
(BOUCHAUD_UI_V1).
"""
from pathlib import Path
import sys
import os

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


# ============================================================================
# BOUCHAUD_C67_LA_TABLE_DE_RELOCATION_DU_DEMARRAGE
# ============================================================================
#
# Un `-static-pie` glibc se relocalise LUI-MEME avant le demarrage normal de
# la libc, dans `_dl_relocate_static_pie`, donc AVANT `main`.
#
# L'ELF de WebWorker mesure sur le run 35912027322 :
#
#     taille            186 808 568 octets
#     .rela.dyn           9 731 304 octets
#     RELACOUNT             405 396 relocations R_X86_64_RELATIVE
#
# ## CE QUI A ETE REFUTE : la variante ET_EXEC
#
# Un `ET_EXEC` n'a aucune relocation de demarrage, et supprimer la phase est
# une falsification bien plus propre que l'optimiser. Cette voie est FERMEE, et
# il faut dire pourquoi, parce que le commentaire qui occupait cette place
# affirmait le contraire.
#
# Il affirmait : « verifie sur un micro-binaire : lie a 0x400000000000 il
# s'execute ». C'etait vrai, et cela ne prouvait rien : le micro-binaire etait
# lie en `-nostdlib`. Il montrait que le CHARGEUR de Bouchaud accepte cette
# adresse, pas que la glibc peut y etre liee.
#
# Avec la glibc statique, l'edition de liens ECHOUE. Mesure localement, gcc 13
# et binutils d'Ubuntu, `-static -no-pie -Wl,-Ttext-segment=0x400000000000` :
#
#     crt1.o: in function `_start':
#     failed to convert GOTPCREL relocation against 'main'; relink with --no-relax
#
# et en ajoutant `--no-relax` comme l'editeur de liens le demande :
#
#     libc.a(printf_buffer_flush.o): relocation truncated to fit:
#     R_X86_64_PLT32 against undefined symbol `__printf_buffer_flush_obstack'
#     libgcc_eh.a(unwind-dw2-fde-dip.o): relocation truncated to fit:
#     R_X86_64_PLT32 against undefined symbol `pthread_cond_wait'
#
# Le mecanisme est structurel, pas un reglage de drapeaux : la glibc statique
# reference des symboles faibles indefinis, qui se resolvent a l'adresse zero.
# Un deplacement relatif de 0x400000000000 vers 0 ne tient pas dans les
# trente-deux bits d'un `R_X86_64_PLT32`. Un `-static-pie` y echappe parce
# qu'il est LIE a la base zero -- c'est le chargement, pas l'edition de liens,
# qui le deplace. Aucun choix d'adresse haute ne contourne cela.
#
# ## CE QUI REMPLACE : RELR
#
# `-Wl,-z,pack-relative-relocs` encode les relocations RELATIVE dans un champ
# de bits `DT_RELR` au lieu d'entrees `Elf64_Rela` de vingt-quatre octets.
# Mesure localement sur un temoin de vingt mille pointeurs :
#
#     defaut   RELASZ = 26 280 octets, RELACOUNT = 1 095
#     relr     RELASZ = 0, RELRSZ = 288 octets, RELRENT = 8
#
# Quatre-vingt-onze fois moins, et les deux binaires s'executent. Extrapole a
# WebWorker, les 9,7 Mio de `.rela.dyn` tomberaient sous la centaine de kio.
#
# ## CE QUE RELR NE FAIT PAS, ET IL FAUT LE DIRE
#
# RELR reduit ce qu'il faut LIRE, pas ce qu'il faut ECRIRE : les 405 396
# ecritures restent. L'experience discrimine donc entre deux couts qu'on
# confondait -- le parcours de la table et l'application des relocations. Si le
# temps ne bouge pas, ce sont les ecritures ; s'il s'effondre, c'etait la
# lecture de la table, c'est-a-dire des fautes de page sur 9,7 Mio.
#
# Contrairement a ET_EXEC, RELR conserve la relocalisation, donc l'ASLR.
#
# ## QUAND LA LANCER
#
# PAS AVANT d'avoir un partage utilisateur/noyau honnete du segment
# `exec_fin -> main`. Le releve `user_ms=92250 sys_ms=671` qui a motive toute
# cette piste etait fausse : le temps du gestionnaire de faute de page tombait
# dans `user_ns` (voir BOUCHAUD_C66). Tant que ce partage n'est pas remesure,
# rien ne dit que le cout est en espace utilisateur.
RELR = os.environ.get("BOUCHAUD_HELPER_RELR", "0") == "1"

if RELR:
    OPTIONS_LIEN = (
        "-static-pie LINKER:-z,pack-relative-relocs "
        "LINKER:--allow-multiple-definition"
    )
else:
    OPTIONS_LIEN = "-static-pie LINKER:--allow-multiple-definition"


def append_runtime_link_options(path: Path, target: str) -> None:
    data = path.read_text()
    marker = f"# Bouchaud runtime link policy for {target}"
    if marker in data:
        # Older generated worktrees may already contain the pre-RPATH version.
        # Replace the whole block so rerunning this script is deterministic.
        start = data.index(marker)
        prefix = data[:start].rstrip()
        data = prefix + "\n"
    block = f'''\n{marker}\nif (BOUCHAUD_PORT)\n    target_link_options({target} PRIVATE {OPTIONS_LIEN})\n\n    # A glibc static PIE relocates itself before normal libc/TLS startup. Its\n    # elf_get_dynamic_info() path asserts that static PIE binaries do not carry\n    # DT_RPATH or DT_RUNPATH. CMake otherwise injects the vcpkg build directory\n    # as RUNPATH even though every dependency is linked statically. Keep the\n    # link-time search path in LIBRARY_PATH/CMAKE_LIBRARY_PATH, but emit no\n    # runtime search path in the Bouchaud executable.\n    set_target_properties({target} PROPERTIES\n        SKIP_BUILD_RPATH TRUE\n        BUILD_WITH_INSTALL_RPATH FALSE\n        INSTALL_RPATH \"\"\n    )\nendif()\n'''
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

# WebDriver est un runtime Bouchaud lui aussi.
# Il doit etre lie en static PIE comme les autres services embarques.
webdriver = root / "Services/WebDriver/CMakeLists.txt"
append_runtime_link_options(webdriver, "WebDriver")

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

print("Bouchaud host/runtime link split applied to", root)
