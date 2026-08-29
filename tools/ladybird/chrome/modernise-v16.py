#!/usr/bin/env python3
from pathlib import Path
import sys

if len(sys.argv) != 2:
    raise SystemExit("usage: modernise-v16.py <ladybird-worktree>")
root=Path(sys.argv[1]).resolve()
header=root / "Services/WebContent/BouchaudChrome.h"
data=header.read_text(encoding="utf-8")
if "BOUCHAUD_CHROME_V16_FONTCONFIG_TYPEFACE" in data:
    print("V16 chrome deja applique")
    raise SystemExit(0)
if "BOUCHAUD_CHROME_V15_REAL_TEXT_SVG_LOADING" not in data:
    raise SystemExit("V16 chrome: V15 transform absent")

inc='#    include <core/SkFont.h>\n'
extra='''#    include <core/SkFont.h>\n#    include <core/SkFontMgr.h>\n#    include <ports/SkFontMgr_fontconfig.h>\n#    include <ports/SkFontScanner_FreeType.h>\n'''
if inc not in data:
    raise SystemExit("V16 chrome: SkFont include absent")
data=data.replace(inc,extra,1)

old='''    static sk_sp<SkTypeface>* typeface = [] {\n        return new sk_sp<SkTypeface>(SkTypeface::MakeFromName("DejaVu Sans", SkFontStyle()));\n    }();\n    if (!*typeface)\n        return false;\n\n    SkFont font(*typeface, 16.0f);\n    font.setEdging(SkFont::Edging::kAntiAlias);'''
new='''    // BOUCHAUD_CHROME_V16_FONTCONFIG_TYPEFACE\n    // Do not use SkTypeface::MakeFromName's process-default backend: the old\n    // artifact could silently fall back to the bitmap atlas. Use the same\n    // FontConfig + FreeType backend as Ladybird page text.\n    static sk_sp<SkFontMgr>* font_manager = [] {\n        return new sk_sp<SkFontMgr>(SkFontMgr_New_FontConfig(nullptr, SkFontScanner_Make_FreeType()));\n    }();\n    static sk_sp<SkTypeface>* typeface = [] {\n        if (!*font_manager)\n            return new sk_sp<SkTypeface>();\n        return new sk_sp<SkTypeface>((*font_manager)->matchFamilyStyle("DejaVu Sans", SkFontStyle()));\n    }();\n    if (!*typeface)\n        return false;\n\n    SkFont font(*typeface, 16.0f);\n    font.setEdging(SkFont::Edging::kAntiAlias);\n    font.setSubpixel(true);'''
if old not in data:
    raise SystemExit("V16 chrome: V15 typeface block absent")
data=data.replace(old,new,1)
header.write_text(data,encoding="utf-8",newline="\n")
print("V16 chrome: FontConfig typeface + subpixel + SVG V15 OK")
