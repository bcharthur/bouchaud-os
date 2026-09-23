#!/usr/bin/env python3
from pathlib import Path
import hashlib, sys
R = Path(__file__).resolve().parents[1]

def need(path, token):
    txt=(R/path).read_text(encoding="utf-8")
    if token not in txt:
        raise SystemExit(f"V13 FAIL {path}: marqueur absent {token}")

for p in [
    "tools/health/images_fixtures.py",
    "tools/health/test_images_fixtures.py",
    "src/kernel/services/registre.rs",
    "src/gui/services.rs",
    "src/main.rs",
    "src/drivers/usb/xhci_active.rs",
    "src/kernel/process/thread/metriques.rs",
    "src/kernel/object/fd.rs",
    "tools/ladybird/prepare-full-browser-host.py",
]:
    need(p, "BOUCHAUD_V13_")

img=(R/"tools/health/images_fixtures.py").read_text(encoding="utf-8")
if "third_party/ladybird/Tests/LibGfx/test-inputs" in img or "_amont(" in img:
    raise SystemExit("V13 FAIL: le banc images depend encore du checkout upstream")
if "crate::gui::services::releve_si_du();" in (R/"src/drivers/usb/xhci_active.rs").read_text(encoding="utf-8"):
    raise SystemExit("V13 FAIL: le sampler Services depend encore de xHCI")

manifest = {
 "jpeg-rgb-checker.jpg":"aa908d27599e54a73b8ee82866bcd2c2d9ef09f6862b5b11eaf60030e6064e84",
 "jpeg-rgb-gradient.jpg":"2d0f67f22353e7917075d870fa2d8be5953d2231b8fa8784fc64f99ecfdc85e0",
 "webp-lossless.webp":"5f8bd924c769c888fe0c98fe302633b871f73a5263fc6970c1ce3848aef7531a",
 "webp-lossy.webp":"bcb33bbe05b1141511427eaed5f46b552a00b0860905aecebdd08a038f0cf9ed",
}
for name, expected in manifest.items():
    data=(R/"tools/health/fixtures/browser-images"/name).read_bytes()
    got=hashlib.sha256(data).hexdigest()
    if got != expected:
        raise SystemExit(f"V13 FAIL fixture {name}: {got} != {expected}")
print("V13 OK: fixtures autonomes, PID recyclable, sampler decouple, /proc dynamique")
