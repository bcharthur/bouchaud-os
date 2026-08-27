#!/usr/bin/env python3
from pathlib import Path
import argparse

parser = argparse.ArgumentParser()
parser.add_argument("--root", required=True)
parser.add_argument("--check", action="store_true")
parser.add_argument("--apply", action="store_true")
args = parser.parse_args()

root = Path(args.root)

def fail(msg):
    raise SystemExit("platform-smp4: " + msg)

def transform(path, replacements):
    if not path.is_file():
        fail(f"absent: {path.relative_to(root)}")
    data = path.read_text()
    for old, new, label in replacements:
        if new in data:
            continue
        if old not in data:
            fail(f"ancre introuvable {label} dans {path.relative_to(root)}")
        data = data.replace(old, new, 1)
    return data

planned = {}

p = root / "tools/ladybird/browser-upstream.sh"
planned[p] = transform(p, [
    (
        'python3 tools/ladybird/prepare-full-browser-host.py "$SRC"\n',
        'python3 tools/ladybird/prepare-full-browser-host.py "$SRC"\n'
        'python3 tools/ladybird/prepare-platform-complete.py "$SRC"\n',
        "prepare platform-complete",
    ),
    (
        "for target in RequestServer ImageDecoder WebWorker Compositor BouchaudBrowserHost; do",
        "for target in RequestServer ImageDecoder WebWorker Compositor WebDriver BouchaudBrowserHost; do",
        "build WebDriver",
    ),
    (
        r' -name Compositor -o -name BouchaudBrowserHost ',
        r' -name Compositor -o -name WebDriver -o -name BouchaudBrowserHost ',
        "copy WebDriver",
    ),
    (
        "for runtime in WebContent RequestServer ImageDecoder WebWorker Compositor BouchaudBrowserHost; do",
        "for runtime in WebContent RequestServer ImageDecoder WebWorker Compositor WebDriver BouchaudBrowserHost; do",
        "readelf WebDriver",
    ),
])

p = root / "run.ps1"
planned[p] = transform(p, [
    (
        "[int]$CpuCount = 1,",
        "[int]$CpuCount = 4,",
        "default 4 vCPU",
    ),
    (
        '"WebWorker",\n            "webcontent-bootstrap",',
        '"WebWorker",\n            "WebDriver",\n            "webcontent-bootstrap",',
        "required WebDriver",
    ),
    (
        'foreach ($service in @("Compositor", "WebWorker", "BouchaudBrowserHost")) {',
        'foreach ($service in @("Compositor", "WebWorker", "WebDriver", "BouchaudBrowserHost")) {',
        "copy WebDriver",
    ),
    (
        '$hostLine = if ($LadybirdInteractif) { \'export BOUCHAUD_BROWSER_HOST=1\' } else { \'echo "Browser Host desactive : regression M9"\' }',
        '$hostLine = if ($LadybirdInteractif) { \'export BOUCHAUD_BROWSER_HOST=1\' } else { \'echo "Browser Host desactive : regression M9"\' }\n'
        '        $timezoneLine = if ($LadybirdInteractif) { \'export BOUCHAUD_TIME_ZONE=Europe/Paris\' } else { \'echo "Timezone Browser Host inactive"\' }\n'
        '        $popupLine = if ($LadybirdInteractif) { \'export BOUCHAUD_ALLOW_POPUPS=1\' } else { \'echo "Popups Browser Host inactifs"\' }',
        "platform environment",
    ),
    (
        '$hostLine,\n                \'desktop\',',
        '$hostLine,\n                $timezoneLine,\n                $popupLine,\n                \'desktop\',',
        "platform autorun lines",
    ),
])

p = root / ".github/workflows/ladybird-native-browser.yml"
planned[p] = transform(p, [
    (
        "          test -x third_party/native-browser-bouchaud/WebWorker\n",
        "          test -x third_party/native-browser-bouchaud/WebWorker\n"
        "          test -x third_party/native-browser-bouchaud/WebDriver\n",
        "verify WebDriver",
    ),
])

p = root / "src/arch/x86_64/mod.rs"
planned[p] = transform(p, [
    (
        "pub mod rtc;\npub mod usermode;",
        "pub mod rtc;\npub mod smp;\npub mod usermode;",
        "smp module",
    ),
])

p = root / "src/main.rs"
planned[p] = transform(p, [
    (
        "    arch::x86_64::init();\n\n    // Calibre le TSC",
        "    arch::x86_64::init();\n"
        "    arch::x86_64::smp::init_probe();\n\n"
        "    // Calibre le TSC",
        "SMP probe boot",
    ),
])

p = root / "src/fs/persistance.rs"
planned[p] = transform(p, [
    (
        "const ENTREES_MAX: usize = 256;",
        "const ENTREES_MAX: usize = 2048;",
        "persistence entries",
    ),
    (
        "const SECTEURS_ZONE: u64 = 16384;",
        "const SECTEURS_ZONE: u64 = 262144;",
        "persistence 128MiB",
    ),
])

p = root / "tools/userland/mkdisk.sh"
planned[p] = transform(p, [
    (
        "ZONE_SECTEURS=16384",
        "ZONE_SECTEURS=262144",
        "mkdisk persistence 128MiB",
    ),
])

if args.check:
    print("Preflight Ladybird platform + SMP4: OK")
    for path in planned:
        print(" ", path.relative_to(root))
    print("Aucun fichier modifie.")
elif args.apply:
    for path, content in planned.items():
        path.write_text(content, newline="\n")
    print("Ladybird platform + SMP4 tracked patches applied.")
else:
    fail("use --check or --apply")
