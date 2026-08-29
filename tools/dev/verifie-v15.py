#!/usr/bin/env python3
from pathlib import Path
import py_compile, re, sys
root=Path(__file__).resolve().parents[2]
required={
    "src/gui/frame_clock.rs":["[FRAME-PERF]","useful_gap_max_ms","fps_arrondi"],
    "src/gui/widgets_v15.rs":["FPS:{:3}","legacy::draw_barre_haute"],
    "src/gui/mod.rs":["widgets_v15.rs","pub mod frame_clock"],
    "src/kernel/debug/journal.rs":["[FPS:","trames utiles"],
    "tools/ladybird/chrome/modernise-v15.py":["draw_browser_text","draw_svg_icon","BOUCHAUD_CHROME_V15_REAL_TEXT_SVG_LOADING"],
    "tools/ladybird/chrome/BouchaudChromeV15Assets.h":["ICON_SIZE","BACK","RELOAD","STOP"],
    "tools/ladybird/verifie-chrome.sh":["modernise-v15.py","BouchaudChromeV15Assets"],
    "tools/perf/run-ladybird-v15.ps1":["ValidateSet(1,4,8)","CpuCount"],
}
for rel,tokens in required.items():
    p=root/rel
    if not p.is_file(): raise SystemExit(f"ABSENT {rel}")
    b=p.read_bytes()
    if b.startswith(b"\xef\xbb\xbf"): raise SystemExit(f"BOM interdit {rel}")
    if b"\r\n" in b: raise SystemExit(f"CRLF interdit {rel}")
    s=b.decode('utf-8')
    for token in tokens:
        if token not in s: raise SystemExit(f"TOKEN absent {rel}: {token}")

# Hz must be gone from runtime telemetry files. Documentation may explain why.
for rel in ["src/gui/frame_clock.rs","src/kernel/debug/journal.rs"]:
    s=(root/rel).read_text(encoding='utf-8')
    if re.search(r"\[Hz:|effective_hz=|target_hz=", s):
        raise SystemExit(f"telemetrie Hz encore active: {rel}")

for rel in ["tools/ladybird/chrome/modernise-v15.py","tools/perf/analyse-v15.py"]:
    py_compile.compile(str(root/rel), doraise=True)

for name in ("back.svg","forward.svg","reload.svg","stop.svg"):
    s=(root/'tools/ladybird/chrome/assets'/name).read_text(encoding='utf-8')
    if '<svg' not in s or '<path' not in s: raise SystemExit(f"SVG invalide {name}")
print("V15 browser UI / FPS / SMP contracts: OK")
