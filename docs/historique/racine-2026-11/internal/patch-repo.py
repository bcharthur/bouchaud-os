#!/usr/bin/env python3
from pathlib import Path
import argparse

parser = argparse.ArgumentParser()
parser.add_argument("repo")
parser.add_argument("--verify-only", action="store_true")
args = parser.parse_args()

root = Path(args.repo)
path = root / "tools/ladybird/browser-upstream.sh"

if not path.is_file():
    raise SystemExit(f"absent: {path}")

data = path.read_text()
original = data

def replace(old, new, label):
    global data
    if new in data:
        return
    if old not in data:
        raise SystemExit(f"ancre introuvable: {label}")
    data = data.replace(old, new, 1)

replace(
    'python3 tools/ladybird/prepare-browser-runtime-link.py "$SRC"\n',
    'python3 tools/ladybird/prepare-browser-runtime-link.py "$SRC"\n'
    'python3 tools/ladybird/prepare-full-browser-host.py "$SRC"\n',
    "ordre prepare-full-browser-host",
)

replace(
    'for target in RequestServer ImageDecoder WebWorker Compositor; do',
    'for target in RequestServer ImageDecoder WebWorker Compositor BouchaudBrowserHost; do',
    "build BouchaudBrowserHost",
)

replace(
    r'''find "$BUILD" -type f \( -name WebContent -o -name RequestServer -o -name ImageDecoder -o -name WebWorker -o -name Compositor \) -perm -111 -exec cp -f {} "$OUT/" \;''',
    r'''find "$BUILD" -type f \( -name WebContent -o -name RequestServer -o -name ImageDecoder -o -name WebWorker -o -name Compositor -o -name BouchaudBrowserHost \) -perm -111 -exec cp -f {} "$OUT/" \;''',
    "copie artefact BrowserHost",
)

replace(
    '[ -x "$OUT/ImageDecoder" ] || { echo "ImageDecoder non produit (images requises)" >&2; exit 1; }\n',
    '[ -x "$OUT/ImageDecoder" ] || { echo "ImageDecoder non produit (images requises)" >&2; exit 1; }\n'
    '[ -x "$OUT/BouchaudBrowserHost" ] || { echo "BouchaudBrowserHost non produit" >&2; exit 1; }\n',
    "validation BrowserHost",
)

replace(
    'for runtime in WebContent RequestServer ImageDecoder WebWorker Compositor; do',
    'for runtime in WebContent RequestServer ImageDecoder WebWorker Compositor BouchaudBrowserHost; do',
    "readelf BrowserHost",
)

if data == original:
    print("browser-upstream.sh: deja pret")
else:
    print("browser-upstream.sh: modifications a appliquer")
    if not args.verify_only:
        path.write_text(data, newline="\n")
        print("browser-upstream.sh: modifie")
