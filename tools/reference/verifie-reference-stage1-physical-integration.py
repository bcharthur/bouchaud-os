#!/usr/bin/env python3
import json, sys
from pathlib import Path
ROOT = Path(__file__).resolve().parents[2]
def read(p): return (ROOT / p).read_text(encoding="utf-8-sig").replace("\r\n","\n")
def req(v,m):
    if not v: print(f"ECHEC: {m}", file=sys.stderr); raise SystemExit(1)
t = json.loads((ROOT/"targets/x86_64-bouchaud_os_uefi.json").read_text(encoding="utf-8-sig"))
req(t.get("relocation-model")=="pic","relocation-model UEFI != pic")
req(t.get("position-independent-executables") is True,"PIE UEFI non active")
req(t.get("static-position-independent-executables") is True,"static PIE UEFI non active")
b=read("tools/reference/build-reference-uefi.ps1")
ib=read("tools/reference/uefi-image-builder/src/main.rs")
g=read("src/platform/pc/reference_gop.rs")
req("x86_64-bouchaud_os_uefi.json" in b,"build UEFI n'utilise pas le target dedie")
req("verifie-reference-uefi-elf.py" in b,"verification ET_DYN absente")
req("config.serial_logging = false;" in ib,"serial bootloader doit etre OFF")
req("config.frame_buffer_logging = true;" in ib,"framebuffer logging bootloader doit etre ON")
for token in ("BOUCHAUD_STAGE1_PHYSICAL_INTEGRATION_V1","render_dashboard_compact","render_dashboard_tiny","render_dashboard_wide","Allocator-tracked","Dynamic VMM","logical reported / 1 active","Installed RAM not queried in Stage 1"):
    req(token in g,f"token integration absent: {token}")
print("REFERENCE_STAGE1_PHYSICAL_INTEGRATION_GUARD_OK")
