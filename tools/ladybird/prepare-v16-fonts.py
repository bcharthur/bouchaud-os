#!/usr/bin/env python3
"""Bouchaud V16: make Ladybird use one real vector font path end-to-end.

The guest installs DejaVu TTF files.  This transform forces WebContent's system
font path through FontConfig/FreeType, puts DejaVu first for CSS generic
families, and teaches PathFontProvider to resolve common missing commercial
family names to DejaVu only when the requested family is not actually loaded.
It also adds nearest-weight matching for the common CSS 500/600 case so a
missing medium face does not fall back to the old bootstrap UI face.
"""
from pathlib import Path
import sys

if len(sys.argv) != 2:
    raise SystemExit("usage: prepare-v16-fonts.py <ladybird-worktree>")
root = Path(sys.argv[1]).resolve()


def replace_once(path: Path, old: str, new: str, label: str):
    data = path.read_text(encoding="utf-8")
    if new in data:
        return
    if old not in data:
        raise SystemExit(f"V16 fonts: anchor absent for {label}: {path}")
    path.write_text(data.replace(old, new, 1), encoding="utf-8", newline="\n")


# 1) Force the pinned WebContent font stack to the FontConfig/FreeType backend.
main = root / "Services/WebContent/main.cpp"
data = main.read_text(encoding="utf-8")
if "BOUCHAUD_V16_FORCE_FONTCONFIG" not in data:
    anchor = "    args_parser.parse(arguments);\n"
    if anchor not in data:
        raise SystemExit("V16 fonts: args_parser.parse anchor absent")
    insertion = (
        "\n#if defined(BOUCHAUD_PORT)\n"
        "    // BOUCHAUD_V16_FORCE_FONTCONFIG\n"
        "    // One rasterisation path for Web text: FreeType + fontconfig over\n"
        "    // the DejaVu files installed by the Bouchaud guest.\n"
        "    force_fontconfig = true;\n"
        "    outln(\"[ladybird-bouchaud] FONTS_V16_FORCE_FONTCONFIG\");\n"
        "#endif\n"
    )
    main.write_text(data.replace(anchor, anchor + insertion, 1), encoding="utf-8", newline="\n")


# 2) Generic family scoring: make the guest's real system faces first-class.
plugin = root / "Libraries/LibWeb/Platform/FontPlugin.cpp"
p = plugin.read_text(encoding="utf-8")
if "BOUCHAUD_V16_DEJAVU_GENERIC" not in p:
    old = '    Vector<FlyString> sans_serif_fallbacks { "Arial"_fly_string,'
    if old not in p:
        raise SystemExit("V16 fonts: sans-serif fallback anchor absent")
    p = p.replace(
        old,
        '    // BOUCHAUD_V16_DEJAVU_GENERIC\n'
        '    Vector<FlyString> sans_serif_fallbacks { "DejaVu Sans"_fly_string, "Arial"_fly_string,',
        1,
    )
    old_mono = '    Vector<FlyString> monospace_fallbacks { "Andale Mono"_fly_string,'
    if old_mono in p:
        p = p.replace(
            old_mono,
            '    Vector<FlyString> monospace_fallbacks { "DejaVu Sans Mono"_fly_string, "Andale Mono"_fly_string,',
            1,
        )
    plugin.write_text(p, encoding="utf-8", newline="\n")


# 3) Named CSS families + CSS weights. Fontconfig aliases alone are not enough:
# PathFontProvider is an exact-family/exact-weight map in this pinned Ladybird.
provider = root / "Libraries/LibGfx/Font/PathFontProvider.cpp"
q = provider.read_text(encoding="utf-8")
if "BOUCHAUD_V16_PATH_FONT_ALIAS" not in q:
    ctor = "PathFontProvider::PathFontProvider() = default;\nPathFontProvider::~PathFontProvider() = default;\n"
    if ctor not in q:
        raise SystemExit("V16 fonts: PathFontProvider ctor anchor absent")
    helper = '''PathFontProvider::PathFontProvider() = default;
PathFontProvider::~PathFontProvider() = default;

// BOUCHAUD_V16_PATH_FONT_ALIAS
// Prefer a genuinely installed requested family. Only when it is absent do we
// map common Web/system names to the TTF families installed by Bouchaud.
static FlyString bouchaud_fallback_family(FlyString const& family)
{
#if defined(BOUCHAUD_PORT)
    if (family == "Google Sans"_fly_string
        || family == "Google Sans Text"_fly_string
        || family == "Google Sans Display"_fly_string
        || family == "Product Sans"_fly_string
        || family == "Roboto"_fly_string
        || family == "Arial"_fly_string
        || family == "Helvetica"_fly_string
        || family == "Helvetica Neue"_fly_string
        || family == "Segoe UI"_fly_string
        || family == "Noto Sans"_fly_string
        || family == "Open Sans"_fly_string
        || family == "Inter"_fly_string
        || family == "SerenitySans"_fly_string)
        return "DejaVu Sans"_fly_string;
    if (family == "Courier New"_fly_string
        || family == "Roboto Mono"_fly_string
        || family == "SF Mono"_fly_string)
        return "DejaVu Sans Mono"_fly_string;
#endif
    return family;
}
'''
    q = q.replace(ctor, helper, 1)

    get_lookup = "    auto it = m_typeface_by_family.find(family);\n"
    if get_lookup not in q:
        raise SystemExit("V16 fonts: get_font family lookup anchor absent")
    q = q.replace(
        get_lookup,
        '''    auto it = m_typeface_by_family.find(family);
#if defined(BOUCHAUD_PORT)
    if (it == m_typeface_by_family.end()) {
        auto fallback_family = bouchaud_fallback_family(family);
        if (fallback_family != family)
            it = m_typeface_by_family.find(fallback_family);
    }
#endif
''',
        1,
    )

    exact = '''    for (auto const& typeface : it->value) {
        if (typeface->weight() == weight && typeface->width() == width && typeface->slope() == slope)
            return typeface->font(point_size, font_variation_settings.value_or_lazy_evaluated([&] { return compute_default_font_variation_settings(weight, width); }), shape_features.value_or_lazy_evaluated([&] { return compute_default_shape_features(); }));
    }

    return nullptr;'''
    if exact not in q:
        raise SystemExit("V16 fonts: exact weight block anchor absent")
    nearest = '''    for (auto const& typeface : it->value) {
        if (typeface->weight() == weight && typeface->width() == width && typeface->slope() == slope)
            return typeface->font(point_size, font_variation_settings.value_or_lazy_evaluated([&] { return compute_default_font_variation_settings(weight, width); }), shape_features.value_or_lazy_evaluated([&] { return compute_default_shape_features(); }));
    }

#if defined(BOUCHAUD_PORT)
    // DejaVu currently exposes regular + bold in the guest. Real sites often
    // ask for 500/600. Choose the nearest compatible face instead of returning
    // nullptr and falling back to the geometric bootstrap font.
    RefPtr<Typeface> nearest;
    unsigned nearest_distance = 0xffffffffu;
    for (auto const& typeface : it->value) {
        if (typeface->width() != width || typeface->slope() != slope)
            continue;
        auto actual = static_cast<unsigned>(typeface->weight());
        auto distance = actual > weight ? actual - weight : weight - actual;
        auto better_tie = nearest
            && ((weight <= 500 && actual < nearest->weight())
                || (weight > 500 && actual > nearest->weight()));
        if (distance < nearest_distance || (distance == nearest_distance && better_tie)) {
            nearest = typeface;
            nearest_distance = distance;
        }
    }
    if (nearest)
        return nearest->font(point_size, font_variation_settings.value_or_lazy_evaluated([&] { return compute_default_font_variation_settings(weight, width); }), shape_features.value_or_lazy_evaluated([&] { return compute_default_shape_features(); }));
#endif

    return nullptr;'''
    q = q.replace(exact, nearest, 1)

    foreach_lookup = "    auto it = m_typeface_by_family.find(family_name);\n"
    if foreach_lookup not in q:
        raise SystemExit("V16 fonts: for_each family lookup anchor absent")
    q = q.replace(
        foreach_lookup,
        '''    auto it = m_typeface_by_family.find(family_name);
#if defined(BOUCHAUD_PORT)
    if (it == m_typeface_by_family.end()) {
        auto fallback_family = bouchaud_fallback_family(family_name);
        if (fallback_family != family_name)
            it = m_typeface_by_family.find(fallback_family);
    }
#endif
''',
        1,
    )
    provider.write_text(q, encoding="utf-8", newline="\n")

print("V16 fonts: FontConfig/FreeType + DejaVu aliases + weight fallback OK")
