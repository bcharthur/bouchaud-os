#!/usr/bin/env python3
'''Modernise le chrome M11 dans l'arbre Ladybird jetable.

V15 ne modifie pas directement `third_party/ladybird-browser-src` dans Git :
il est recree a chaque build. Ce script est appele par `verifie-chrome.sh`
apres les preparateurs M11, juste avant le build CMake.
'''
from pathlib import Path
import re
import shutil
import sys

if len(sys.argv) != 2:
    raise SystemExit("usage: modernise-v15.py <ladybird-browser-src>")

root = Path(sys.argv[1]).resolve()
header = root / "Services/WebContent/BouchaudChrome.h"
if not header.is_file():
    raise SystemExit(f"V15: BouchaudChrome.h absent: {header}")

repo = Path(__file__).resolve().parents[3]
asset_src = repo / "tools/ladybird/chrome/BouchaudChromeV15Assets.h"
asset_dst = header.with_name("BouchaudChromeV15Assets.h")
shutil.copyfile(asset_src, asset_dst)

data = header.read_text(encoding="utf-8")

# BouchaudChrome est un en-tete inclus directement par les objets du service
# WebContent. LibGfx utilise deja Skia, mais le lien est PRIVATE : ses chemins
# d'inclusion ne se propagent donc pas au consommateur. Declarer Skia sur le
# vrai target qui compile l'en-tete fournit a la fois include/skia et la
# dependance de lien statique, sans injecter un chemin propre au runner.
cmake = root / "Services/WebContent/CMakeLists.txt"
cmake_data = cmake.read_text(encoding="utf-8")
skia_link = "target_link_libraries(webcontentservice PRIVATE skia)"
if skia_link not in cmake_data:
    cmake_anchor = "target_link_libraries(webcontentservice PRIVATE SDL3::SDL3)\n"
    if cmake_anchor not in cmake_data:
        raise SystemExit("V15: ancre CMake webcontentservice/SDL3 introuvable")
    cmake_data = cmake_data.replace(
        cmake_anchor,
        cmake_anchor
        + "if (BOUCHAUD_PORT)\n"
        + f"    {skia_link}\n"
        + "endif()\n",
        1,
    )
    cmake.write_text(cmake_data, encoding="utf-8", newline="\n")

if "BOUCHAUD_CHROME_V15_REAL_TEXT_SVG_LOADING" in data:
    print("V15 chrome deja modernise; dependance Skia WebContent verifiee")
    raise SystemExit(0)

# 1) Skia + image assets. Ils restent sous BOUCHAUD_PORT : le chrome n'existe
# pas dans la build upstream normale.
needle = '#include "BouchaudAtlas.h"\n'
if needle not in data:
    raise SystemExit("V15: include BouchaudAtlas.h introuvable")
data = data.replace(needle, needle + '#include "BouchaudChromeV15Assets.h"\n', 1)

needle = '#    include <LibGfx/Bitmap.h>\n'
if needle not in data:
    raise SystemExit("V15: include LibGfx/Bitmap.h introuvable")
skia = r'''#    include <LibGfx/Bitmap.h>
#    include <core/SkBitmap.h>
#    include <core/SkCanvas.h>
#    include <core/SkFont.h>
#    include <core/SkImageInfo.h>
#    include <core/SkPaint.h>
#    include <core/SkRect.h>
#    include <core/SkTypeface.h>
'''
data = data.replace(needle, skia, 1)

# 2) Helpers juste avant draw_ui_text, une fois Canvas/fill_rect definis.
#
# L'ancre etait `draw_toolbar`. Elle ne peut plus l'etre : `draw_ui_text` est
# desormais le point unique par lequel passe TOUT le texte d'interface du
# chrome -- barre d'adresse, bulle de survol, barre de recherche, menu -- et
# c'est son corps que l'etape 4 remplace par le rendu Skia. Les aides doivent
# donc etre definies avant lui, pas seulement avant `draw_toolbar`.
anchor = '/// Le texte d\'interface du chrome, en UN SEUL point de passage.\n'
if anchor not in data:
    raise SystemExit("V15: draw_ui_text introuvable")
helpers = r'''// BOUCHAUD_CHROME_V15_REAL_TEXT_SVG_LOADING
//
// Le document et le chrome utilisent desormais le meme rasteriseur Skia pour
// le texte visible de la barre d'adresse. L'ancien atlas DejaVu reste le
// fallback et continue de servir a la mesure/caret : aucune panne de fontconfig
// ne peut rendre la navigation inutilisable.
inline bool draw_browser_text(Canvas const& canvas, int x, int y, StringView text, u32 color, int max_width)
{
    if (!canvas.base || canvas.width <= 0 || canvas.height <= 0 || canvas.stride <= 0 || max_width <= 0)
        return false;

    SkBitmap bitmap;
    auto info = SkImageInfo::Make(canvas.width, canvas.height, kBGRA_8888_SkColorType, kOpaque_SkAlphaType);
    if (!bitmap.installPixels(info, canvas.base, static_cast<size_t>(canvas.stride)))
        return false;

    // Le pointeur est volontairement alloue une fois et jamais detruit :
    // Ladybird interdit les destructeurs statiques (-Wexit-time-destructors).
    static sk_sp<SkTypeface>* typeface = [] {
        return new sk_sp<SkTypeface>(SkTypeface::MakeFromName("DejaVu Sans", SkFontStyle()));
    }();
    if (!*typeface)
        return false;

    SkFont font(*typeface, 16.0f);
    font.setEdging(SkFont::Edging::kAntiAlias);
    SkPaint paint;
    paint.setAntiAlias(true);
    paint.setColor(SkColorSetARGB(0xff, (color >> 16) & 0xff, (color >> 8) & 0xff, color & 0xff));

    SkCanvas sk_canvas(bitmap);
    sk_canvas.save();
    sk_canvas.clipRect(SkRect::MakeXYWH(static_cast<float>(x), static_cast<float>(y),
        static_cast<float>(max_width), 22.0f));
    sk_canvas.drawSimpleText(text.characters_without_null_termination(), text.length(),
        SkTextEncoding::kUTF8, static_cast<float>(x), static_cast<float>(y + 17), font, paint);
    sk_canvas.restore();
    return true;
}

inline void blend_icon_pixel(u32& destination, u32 source, unsigned alpha)
{
    if (alpha == 0)
        return;
    auto blend = [alpha](u32 src, u32 dst) -> u32 {
        return (src * alpha + dst * (255 - alpha) + 127) / 255;
    };
    auto r = blend((source >> 16) & 0xff, (destination >> 16) & 0xff);
    auto g = blend((source >> 8) & 0xff, (destination >> 8) & 0xff);
    auto b = blend(source & 0xff, destination & 0xff);
    destination = (r << 16) | (g << 8) | b;
}

inline void draw_svg_icon(Canvas const& canvas, int x, int y, unsigned char const* mask, u32 color)
{
    using namespace BouchaudChromeV15Assets;
    for (int iy = 0; iy < ICON_SIZE; ++iy) {
        auto py = y + iy;
        if (py < 0 || py >= canvas.height)
            continue;
        auto* row = canvas.row(py);
        for (int ix = 0; ix < ICON_SIZE; ++ix) {
            auto px = x + ix;
            if (px < 0 || px >= canvas.width)
                continue;
            blend_icon_pixel(row[px], color, mask[iy * ICON_SIZE + ix]);
        }
    }
}

'''
data = data.replace(anchor, helpers + anchor, 1)

# 3) Les trois boutons ne sont plus des glyphes de police. Les fichiers SVG
# restent visibles/revisables dans le depot; le header V15 contient leur masque
# antialiase pre-rasterise pour ne pas embarquer un parseur SVG dans WebContent.
pattern = re.compile(r'''    auto draw_button = \[&\]\(Button const& button, bool active\) \{\n        fill_rect\(canvas, button\.x, button_top, button\.width, button_height,\n            active \? color_button : color_button_off\);\n        auto glyph_x = button\.x \+ \(button\.width - glyph_width \* 2\) / 2;\n        auto glyph_y = button_top \+ \(button_height - glyph_height \* 2\) / 2;\n        draw_glyph\(canvas, glyph_x, glyph_y, button\.glyph\[0\],\n            active \? color_glyph : color_glyph_off, 2\);\n    \};''')
replacement = r'''    auto draw_button = [&](Button const& button, bool active) {
        fill_rect(canvas, button.x, button_top, button.width, button_height,
            active ? color_button : color_button_off);
        auto icon_x = button.x + (button.width - BouchaudChromeV15Assets::ICON_SIZE) / 2;
        auto icon_y = button_top + (button_height - BouchaudChromeV15Assets::ICON_SIZE) / 2;
        auto const* icon = BouchaudChromeV15Assets::BACK;
        if (button.x == forward_button().x)
            icon = BouchaudChromeV15Assets::FORWARD;
        else if (button.x == reload_button().x)
            icon = s.loading ? BouchaudChromeV15Assets::STOP : BouchaudChromeV15Assets::RELOAD;
        draw_svg_icon(canvas, icon_x, icon_y, icon, active ? color_glyph : color_glyph_off);
    };'''
data, n = pattern.subn(replacement, data, count=1)
if n != 1:
    raise SystemExit(f"V15: bloc draw_button inattendu ({n})")

# 4) Tout le texte d'interface : vrai texte Skia, avec repli sur l'atlas si la
# police est absente. Un seul corps a remplacer, donc un seul endroit ou une
# substitution peut manquer sa cible -- voir le commentaire de `draw_ui_text`.
old = '''inline void draw_ui_text(Canvas const& canvas, int x, int y, StringView texte, u32 couleur, int largeur_max)
{
    draw_text(canvas, x, y + 1, texte, couleur, 2, largeur_max);
}'''
new = '''inline void draw_ui_text(Canvas const& canvas, int x, int y, StringView texte, u32 couleur, int largeur_max)
{
    if (draw_browser_text(canvas, x, y, texte, couleur, largeur_max))
        return;
    draw_text(canvas, x, y + 1, texte, couleur, 2, largeur_max);
}'''
if old not in data:
    raise SystemExit("V15: corps de draw_ui_text introuvable")
data = data.replace(old, new, 1)

# 5) Etat de chargement : stop dans le bouton + ligne bleue et point d'etat.
# Pas de faux pourcentage : tant que Ladybird ne publie pas une progression
# reseau/document, une barre indeterminee est plus exacte qu'un 42% invente.
status_pattern = re.compile(r'''    // Etat du chargement, a droite, en petit\.\n    auto status_text = s\.loading \? ByteString \{ "chargement\.\.\." \} : s\.status;\n    auto status_width = text_width\(status_text\.view\(\), 1\);\n    auto status_x = canvas\.width - margin - status_width;\n    if \(status_x > field_x \+ field_w \+ 4\)\n        draw_text\(canvas, status_x, toolbar_height - glyph_height - 3, status_text\.view\(\), color_glyph_off, 1, status_width\);''')
status_new = r'''    // Indicateur visuel : bleu = navigation en cours, vert = page stabilisee.
    // La ligne n'affiche volontairement aucun pourcentage fictif.
    auto status_color = s.loading ? 0x3b82f6u : 0x22c55eu;
    fill_rect(canvas, field_x + field_w - 9, button_top + 10, 5, 5, status_color);
    if (s.loading)
        fill_rect(canvas, field_x, toolbar_height - 3, field_w, 3, status_color);'''
data, n = status_pattern.subn(status_new, data, count=1)
if n != 1:
    raise SystemExit(f"V15: bloc etat chargement inattendu ({n})")

header.write_text(data, encoding="utf-8", newline="\n")
print("V15 chrome: texte Skia + SVG + loading indicator appliques")
