#!/usr/bin/env python3
from pathlib import Path
import xml.etree.ElementTree as ET
root=Path(__file__).resolve().parents[2]
need={
"src/kernel/sync/reveil/attente.rs":["expected_depth","INTERFACE_DETACHED_NESTED","profondeur_locale() > 0"],
"src/gui/desktop_bkl/scope.rs":["accepte_imbrique","NESTED_SCOPES","suspend_for_schedule"],
"src/kernel/process/thread/faute_cluster.rs":["ZERO_CLUSTER_MAX_PAGES","zero_fault_cluster_stats","FAULT_CLUSTER_MAX_PAGES: u64 = 16","let window: u64 ="],
"src/kernel/process/thread/faute_memoire.rs":["fault_cluster_after_zero"],
"tools/ladybird/chrome/modernise-v16.py":["BOUCHAUD_CHROME_V16_FONTCONFIG_TYPEFACE","SkFontMgr_New_FontConfig"],
"tools/ladybird/prepare-v16-fonts.py":["FONTS_V16_FORCE_FONTCONFIG","BOUCHAUD_V16_PATH_FONT_ALIAS","nearest_distance","DejaVu Sans"],
# `perf/gui-event-driven` a ete fusionnee dans main puis supprimee : epingler
# une branche morte ne protege rien. On garde le marqueur de capacite UI, qui
# est le vrai contrat de ce workflow.
# Le workflow V16 dedie a ete replie dans `ladybird-native-browser.yml`,
# qui porte maintenant le marqueur de capacite UI. Le contrat suit le code.
".github/workflows/ladybird-native-browser.yml":["V16_UI_CAPABLE"],
"tools/perf/run-ladybird-v16.ps1":["V16_UI_CAPABLE","Get-V16Artifact"],
}
for rel,tokens in need.items():
    p=root/rel
    if not p.is_file(): raise SystemExit(f"V16 missing {rel}")
    s=p.read_text(encoding="utf-8")
    for t in tokens:
        if t not in s: raise SystemExit(f"V16 contract missing {t} in {rel}")
    if '\r' in s: raise SystemExit(f"V16 CRLF forbidden in drop-in: {rel}")
ET.parse(root/"tools/ladybird/fontconfig/fonts.conf")
# include fragments must never start with inner doc comments
for rel in ["src/kernel/sync/reveil/attente.rs","src/kernel/sync/reveil/etat.rs","src/kernel/sync/reveil/diagnostic.rs","src/gui/desktop_bkl/scope.rs","src/kernel/process/thread/faute_cluster.rs"]:
    if (root/rel).read_text(encoding="utf-8").lstrip().startswith("//!"):
        raise SystemExit(f"V16 include fragment uses //!: {rel}")
print("V16 typography + fluidity contracts: OK")
