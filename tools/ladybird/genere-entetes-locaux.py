#!/usr/bin/env python3
"""Genere, sans CMake ni vcpkg, les en-tetes que Ladybird produit au build.

BOUCHAUD_LB_ENTETES_GENERES_LOCAUX_V1

Pourquoi : un frontend (`UI/Bouchaud`) inclut `LibWebView/ViewImplementation.h`,
qui tire les points d'entree IPC (`WebContentServerEndpoint.h`...), les
bindings IDL (`LibWeb/Bindings/*.h`) et les enumerations CSS. Tous sont
GENERES. Sans eux, la seule verification possible etait le build CI complet
(~95 min froid). Les generateurs upstream sont des scripts Python : on les
appelle ici exactement comme `Meta/CMake/*.cmake` les appelle.

  python3 tools/ladybird/genere-entetes-locaux.py <arbre-ladybird> <sortie>

`<sortie>` s'ajoute ensuite en `-I` (et `<sortie>/Libraries`,
`<sortie>/Services`) a une compilation `-fsyntax-only`. Ce script ne remplace
pas le build : il ne produit ni Skia, ni ICU, ni les objets -- seulement les
en-tetes dont la SEMANTIQUE du code C++ depend.
"""
import re
import subprocess
import sys
from pathlib import Path


def lance(arbre: Path, script: str, *args: str) -> None:
    gen = arbre / "Meta/Generators" / script
    r = subprocess.run([sys.executable, str(gen), *args], capture_output=True, text=True)
    if r.returncode != 0:
        raise SystemExit(f"{script} : echec\n{r.stderr[-2000:]}")


def css(arbre: Path, sortie: Path) -> list[Path]:
    css = arbre / "Libraries/LibWeb/CSS"
    o = sortie / "Libraries/LibWeb/CSS"
    (o / "Parser").mkdir(parents=True, exist_ok=True)
    simples = [
        ("generate_libweb_css_descriptors.py", "DescriptorID", ["-j", css / "Descriptors.json"]),
        ("generate_libweb_css_enums.py", "Enums", ["-j", css / "Enums.json"]),
        ("generate_libweb_css_environment_variables.py", "EnvironmentVariable", ["-j", css / "EnvironmentVariables.json"]),
        ("generate_libweb_css_math_functions.py", "MathFunctions", ["-j", css / "MathFunctions.json"]),
        ("generate_libweb_css_media_feature_id.py", "MediaFeatureID", ["-j", css / "MediaFeatures.json"]),
        ("generate_libweb_css_property_id.py", "PropertyID",
         ["-j", css / "Properties.json", "-e", css / "Enums.json", "-g", css / "LogicalPropertyGroups.json"]),
        ("generate_libweb_css_pseudo_class.py", "PseudoClass", ["-j", css / "PseudoClasses.json"]),
        ("generate_libweb_css_pseudo_element.py", "PseudoElement", ["-j", css / "PseudoElements.json"]),
        ("generate_libweb_css_transform_functions.py", "TransformFunctions", ["-j", css / "TransformFunctions.json"]),
        ("generate_libweb_css_value_types_parsing.py", "Parser/GeneratedValueTypesParsing",
         ["-j", css / "ValueTypes.json", "-u", css / "Units.json"]),
        ("generate_libweb_css_units.py", "Units", ["-j", css / "Units.json"]),
        ("generate_libweb_css_keyword.py", "Keyword", ["-j", css / "Keywords.json"]),
    ]
    for script, nom, args in simples:
        lance(arbre, script, "-h", str(o / f"{nom}.h"), "-c", str(o / f"{nom}.cpp"), *map(str, args))
    idl = []
    for script, nom, json in (
        ("generate_libweb_css_numeric_factory_methods.py", "GeneratedCSSNumericFactoryMethods", "Units.json"),
        ("generate_libweb_css_style_properties.py", "GeneratedCSSStyleProperties", "Properties.json"),
    ):
        lance(arbre, script, "-h", str(o / f"{nom}.h"), "-c", str(o / f"{nom}.cpp"),
              "-i", str(o / f"{nom}.idl"), "-j", str(css / json))
        idl.append(o / f"{nom}.idl")
    return idl


def html_webgl(arbre: Path, sortie: Path) -> None:
    w = arbre / "Libraries/LibWeb"
    o = sortie / "Libraries/LibWeb"
    for d in ("HTML/Parser", "WebGL"):
        (o / d).mkdir(parents=True, exist_ok=True)
    lance(arbre, "generate_libweb_html_named_character_references.py",
          "-h", str(o / "HTML/Parser/NamedCharacterReferences.h"),
          "-c", str(o / "HTML/Parser/NamedCharacterReferences.cpp"),
          "-j", str(w / "HTML/Parser/Entities.json"))
    lance(arbre, "generate_dom_tree.py",
          "-h", str(o / "HTML/MediaControlsDOM.h"), "-c", str(o / "HTML/MediaControlsDOM.cpp"),
          "-i", str(w / "HTML/MediaControls.html"), "-s", "MediaControlsDOM", "-n", "Web::HTML",
          "--html-tags", str(w / "HTML/TagNames.h"), "--html-attributes", str(w / "HTML/AttributeNames.h"),
          "--svg-tags", str(w / "SVG/TagNames.h"), "--svg-attributes", str(w / "SVG/AttributeNames.h"))
    (o / "ARIA").mkdir(parents=True, exist_ok=True)
    lance(arbre, "generate_libweb_aria_roles.py", "-h", str(o / "ARIA/AriaRoles.h"),
          "-c", str(o / "ARIA/AriaRoles.cpp"), "-j", str(w / "ARIA/AriaRoles.json"))
    for script, nom in (("generate_libweb_webgl_functions.py", "GLFunctions"),
                        ("generate_libweb_webgl_commands.py", "WebGLCommands"),
                        ("generate_libweb_webgl_proxy.py", "WebGLContextProxy")):
        lance(arbre, script, "-h", str(o / f"WebGL/{nom}.h"), "-c", str(o / f"WebGL/{nom}.cpp"),
              "-j", str(w / "WebGL/GLFunctions.json"))


def bindings(arbre: Path, sortie: Path, idl_generes: list[Path]) -> None:
    libweb = arbre / "Libraries/LibWeb"
    generes = {p.stem: p for p in idl_generes}
    fichiers = []
    for m in re.finditer(r"^libweb_js_bindings\(([^\s)]+)", (libweb / "idl_files.cmake").read_text(), re.M):
        classe = m.group(1)
        base = classe.rsplit("/", 1)[-1]
        fichiers.append(str(generes.get(base, libweb / f"{classe}.idl")))
    o = sortie / "Libraries/LibWeb/Bindings"
    o.mkdir(parents=True, exist_ok=True)
    lance(arbre, "generate_libweb_bindings.py", "-o", str(o), *fichiers)


def ipc(arbre: Path, sortie: Path) -> int:
    n = 0
    for cm in list((arbre / "Libraries").glob("*/CMakeLists.txt")):
        for m in re.finditer(r"compile_ipc\(([^\s)]+)\s+([^\s)]+)\)", cm.read_text()):
            src, dst = m.group(1), m.group(2)
            src = src.replace("${LADYBIRD_SOURCE_DIR}", str(arbre))
            src_p = Path(src) if Path(src).is_absolute() else (cm.parent / src)
            if "${CMAKE_BINARY_DIR}" in dst:
                dst_p = sortie / dst.replace("${CMAKE_BINARY_DIR}/", "")
            else:
                dst_p = (sortie / cm.parent.relative_to(arbre) / dst).resolve()
            dst_p.parent.mkdir(parents=True, exist_ok=True)
            lance(arbre, "generate_ipc_definitions.py", "--input", str(src_p.resolve()), "--output", str(dst_p))
            n += 1
    return n


def exports(arbre: Path, sortie: Path) -> int:
    """`LibX/Export.h` est produit par CMake (GenerateExportHeader) : une
    macro de visibilite par bibliotheque. Vide ici, comme en edition statique."""
    n = 0
    for lib in sorted((arbre / "Libraries").glob("Lib*")):
        macros = set()
        for f in lib.rglob("*.h"):
            macros.update(re.findall(r"\b([A-Z][A-Z0-9]*_API)\b", f.read_text(errors="ignore")))
        d = sortie / "Libraries" / lib.name
        d.mkdir(parents=True, exist_ok=True)
        corps = "".join(f"#ifndef {m}\n#define {m}\n#endif\n" for m in sorted(macros))
        (d / "Export.h").write_text("#pragma once\n" + corps)
        n += 1
    return n


def debug(arbre: Path, sortie: Path) -> None:
    """`AK/Debug.h` : les drapeaux `*_DEBUG`, tous a zero."""
    drapeaux = set()
    for racine in ("AK", "Libraries", "Services"):
        for f in (arbre / racine).rglob("*.[ch]*"):
            drapeaux.update(re.findall(r"\b([A-Z0-9_]+_DEBUG)\b", f.read_text(errors="ignore")))
    (sortie / "AK").mkdir(parents=True, exist_ok=True)
    (sortie / "AK/Debug.h").write_text("#pragma once\n" + "".join(
        f"#ifndef {d}\n#define {d} 0\n#endif\n" for d in sorted(drapeaux)))


CRATES = {
    "libgfx_rust": "LibGfx", "libjs_rust": "LibJS", "libweb_rust": "LibWeb",
    "libweb_content_blocker_rust": "LibWeb", "libweb_html_tokenizer": "LibWeb",
    "libunicode_rust": "LibUnicode", "libtextcodec_rust": "LibTextCodec",
    "libregex_rust": "LibRegex", "liburl_rust": "LibURL",
}


def rust_ffi(arbre: Path, sortie: Path, cible: Path) -> int:
    """Les en-tetes cbindgen sont ecrits par les `build.rs` des crates :
    `cargo check` les execute sans compiler le code Rust en objets."""
    import os
    import shutil
    env = dict(os.environ, CARGO_TARGET_DIR=str(cible))
    r = subprocess.run(["cargo", "check", "--workspace", "--target", "x86_64-unknown-linux-gnu", "-q"],
                       cwd=arbre, env=env, capture_output=True, text=True)
    if r.returncode != 0:
        raise SystemExit(f"cargo check : echec\n{r.stderr[-2000:]}")
    n = 0
    for out in (cible / "x86_64-unknown-linux-gnu/debug/build").glob("*/out"):
        crate = out.parent.name.rsplit("-", 1)[0]
        lib = CRATES.get(crate)
        if lib is None:
            continue
        for h in out.rglob("*.h"):
            d = sortie / "Libraries" / lib / h.relative_to(out)
            d.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(h, d)
            n += 1
    return n


def main() -> int:
    if len(sys.argv) != 3:
        print(__doc__)
        return 2
    arbre, sortie = Path(sys.argv[1]).resolve(), Path(sys.argv[2]).resolve()
    sortie.mkdir(parents=True, exist_ok=True)
    idl = css(arbre, sortie)
    html_webgl(arbre, sortie)
    bindings(arbre, sortie, idl)
    n = ipc(arbre, sortie)
    e = exports(arbre, sortie)
    debug(arbre, sortie)
    r = rust_ffi(arbre, sortie, sortie.parent / (sortie.name + "-cargo"))
    print(f"ENTETES_GENERES_OK css=1 bindings=1 ipc={n} exports={e} rust_ffi={r} sortie={sortie}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
