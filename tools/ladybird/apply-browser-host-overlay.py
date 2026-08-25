#!/usr/bin/env python3
'''
Installateur non destructif du patch Browser Host.

Aucun git add/commit/reset/clean.
Sauvegarde horodatee avant toute modification.
'''

from pathlib import Path
from datetime import datetime
import argparse
import shutil


def fail(message: str) -> None:
    raise SystemExit("ERREUR BrowserHost patch: " + message)


def replace_once(text: str, old: str, new: str, label: str) -> str:
    if new in text:
        return text
    if old not in text:
        fail(f"ancre introuvable: {label}. Le checkout ne correspond probablement pas a PR #202.")
    return text.replace(old, new, 1)


def patch_browser_upstream(path: Path) -> str:
    text = path.read_text()

    old = 'python3 tools/ladybird/prepare-browser-runtime-link.py "$SRC"\n'
    new = old + 'python3 tools/ladybird/prepare-full-browser-host.py "$SRC"\n'
    text = replace_once(text, old, new, "appel prepare-full-browser-host")

    old_loop = 'for target in RequestServer ImageDecoder WebWorker Compositor; do'
    new_loop = 'for target in RequestServer ImageDecoder WebWorker Compositor BouchaudBrowserHost; do'
    text = replace_once(text, old_loop, new_loop, "boucle build helpers")

    old_find = r'find "$BUILD" -type f \( -name WebContent -o -name RequestServer -o -name ImageDecoder -o -name WebWorker -o -name Compositor \) -perm -111 -exec cp -f {} "$OUT/" \;'
    new_find = r'find "$BUILD" -type f \( -name WebContent -o -name RequestServer -o -name ImageDecoder -o -name WebWorker -o -name Compositor -o -name BouchaudBrowserHost \) -perm -111 -exec cp -f {} "$OUT/" \;'
    text = replace_once(text, old_find, new_find, "find artefacts")

    anchor = '[ -x "$OUT/ImageDecoder" ] || { echo "ImageDecoder non produit (images requises)" >&2; exit 1; }\n'
    new_required = anchor + (
        '[ -x "$OUT/Compositor" ] || { echo "Compositor non produit (Browser Host requis)" >&2; exit 1; }\n'
        '[ -x "$OUT/WebWorker" ] || { echo "WebWorker non produit (Browser Host requis)" >&2; exit 1; }\n'
        '[ -x "$OUT/BouchaudBrowserHost" ] || { echo "BouchaudBrowserHost non produit" >&2; exit 1; }\n'
    )
    text = replace_once(text, anchor, new_required, "artefacts BrowserHost requis")

    old_runtime = 'for runtime in WebContent RequestServer ImageDecoder WebWorker Compositor; do'
    new_runtime = 'for runtime in WebContent RequestServer ImageDecoder WebWorker Compositor BouchaudBrowserHost; do'
    text = replace_once(text, old_runtime, new_runtime, "readelf helpers")

    return text


def patch_run_ps1(path: Path) -> str:
    text = path.read_text()

    old_required = r'''    if ($IsLadybirdM8) {
        $RequiredLadybirdFiles = @(
            "WebContent",
            "ImageDecoder",
            "webcontent-bootstrap"
        )
    }
    else {
        $RequiredLadybirdFiles = @(
            "WebContent",
            "RequestServer",
            "ImageDecoder",
            "webcontent-bootstrap",
            "M9_CAPABLE"
        )
    }'''
    new_required = r'''    if ($IsLadybirdM8) {
        $RequiredLadybirdFiles = @(
            "WebContent",
            "ImageDecoder",
            "webcontent-bootstrap"
        )
    }
    elseif ($LadybirdInteractif) {
        $RequiredLadybirdFiles = @(
            "BouchaudBrowserHost",
            "WebContent",
            "RequestServer",
            "ImageDecoder",
            "Compositor",
            "WebWorker",
            "webcontent-bootstrap",
            "M9_CAPABLE"
        )
    }
    else {
        $RequiredLadybirdFiles = @(
            "WebContent",
            "RequestServer",
            "ImageDecoder",
            "webcontent-bootstrap",
            "M9_CAPABLE"
        )
    }'''
    text = replace_once(text, old_required, new_required, "RequiredLadybirdFiles")

    image_copy = r'''    Copy-Item `
        (Join-Path $NativeBrowserDir "ImageDecoder") `
        (Join-Path $LadybirdLibexec "ImageDecoder")
'''
    extra_copy = image_copy + r'''
    if ($LadybirdInteractif) {
        foreach ($service in @("Compositor", "WebWorker", "BouchaudBrowserHost")) {
            Copy-Item `
                (Join-Path $NativeBrowserDir $service) `
                (Join-Path $LadybirdLibexec $service)
        }
    }
'''
    text = replace_once(text, image_copy, extra_copy, "copie services BrowserHost")

    old_entry = r'''    Copy-Item `
        (Join-Path $NativeBrowserDir "webcontent-bootstrap") `
        (Join-Path $ScenarioDir "bo-navigateur")'''
    new_entry = r'''    $BrowserEntry = if ($LadybirdInteractif) {
        "BouchaudBrowserHost"
    }
    else {
        "webcontent-bootstrap"
    }

    Copy-Item `
        (Join-Path $NativeBrowserDir $BrowserEntry) `
        (Join-Path $ScenarioDir "bo-navigateur")'''
    text = replace_once(text, old_entry, new_entry, "entree /bo-navigateur")

    chrome_anchor = r'''        $chromeLine = if ($LadybirdChrome) { 'export BOUCHAUD_M11=1' } else { 'echo "M11 desactive : capture unique, sans entrees"' }

        $autorun = @('''
    chrome_new = r'''        $chromeLine = if ($LadybirdChrome) { 'export BOUCHAUD_M11=1' } else { 'echo "M11 desactive : capture unique, sans entrees"' }
        $hostLine = if ($LadybirdInteractif) { 'export BOUCHAUD_BROWSER_HOST=1' } else { 'echo "Browser Host desactive : regression M9"' }

        $autorun = @('''
    text = replace_once(text, chrome_anchor, chrome_new, "hostLine autorun")

    url_block = r'''                "export BOUCHAUD_M9_URL=$(ConvertTo-ShellSingleQuoted $LadybirdUrl)",
                $chromeLine,
                'desktop','''
    url_new = r'''                "export BOUCHAUD_M9_URL=$(ConvertTo-ShellSingleQuoted $LadybirdUrl)",
                $chromeLine,
                $hostLine,
                'desktop','''
    text = replace_once(text, url_block, url_new, "export BOUCHAUD_BROWSER_HOST")

    old_exec = r'''executables = {
    "bo-navigateur",
    "usr/libexec/ladybird/WebContent",
    "usr/libexec/ladybird/RequestServer",
    "usr/libexec/ladybird/ImageDecoder",
    "usr/libexec/ladybird/webcontent-bootstrap",
}'''
    new_exec = r'''executables = {
    "bo-navigateur",
    "usr/libexec/ladybird/BouchaudBrowserHost",
    "usr/libexec/ladybird/WebContent",
    "usr/libexec/ladybird/RequestServer",
    "usr/libexec/ladybird/ImageDecoder",
    "usr/libexec/ladybird/Compositor",
    "usr/libexec/ladybird/WebWorker",
    "usr/libexec/ladybird/webcontent-bootstrap",
}'''
    text = replace_once(text, old_exec, new_exec, "bits execution BrowserHost")

    return text


def patch_workflow(path: Path) -> str:
    text = path.read_text()
    anchor = r'''          test -x third_party/native-browser-bouchaud/WebContent
          test -x third_party/native-browser-bouchaud/RequestServer
          test -x third_party/native-browser-bouchaud/webcontent-bootstrap
          test -f third_party/native-browser-bouchaud/M9_CAPABLE'''
    new = r'''          test -x third_party/native-browser-bouchaud/BouchaudBrowserHost
          test -x third_party/native-browser-bouchaud/WebContent
          test -x third_party/native-browser-bouchaud/RequestServer
          test -x third_party/native-browser-bouchaud/ImageDecoder
          test -x third_party/native-browser-bouchaud/Compositor
          test -x third_party/native-browser-bouchaud/WebWorker
          test -x third_party/native-browser-bouchaud/webcontent-bootstrap
          test -f third_party/native-browser-bouchaud/M9_CAPABLE'''
    return replace_once(text, anchor, new, "verification artefacts workflow")



def preflight(root: Path) -> None:
    """Validate the target checkout without writing anything."""
    expected = [
        root / "tools/ladybird/browser-upstream.sh",
        root / "run.ps1",
        root / ".github/workflows/ladybird-native-browser.yml",
        root / "tools/ladybird/prepare-browser-host.py",
        root / "tools/ladybird/prepare-m11-chrome.py",
        root / "tools/ladybird/prepare-browser-runtime-link.py",
    ]
    for path in expected:
        if not path.is_file():
            fail(f"preflight: {path.relative_to(root)} absent")

    # These calls perform every anchor replacement in memory. If the checkout
    # diverges from PR #202, they fail here before any backup/write.
    planned = {
        root / "tools/ladybird/browser-upstream.sh":
            patch_browser_upstream(root / "tools/ladybird/browser-upstream.sh"),
        root / "run.ps1":
            patch_run_ps1(root / "run.ps1"),
        root / ".github/workflows/ladybird-native-browser.yml":
            patch_workflow(root / ".github/workflows/ladybird-native-browser.yml"),
    }

    # Make sure the simulated result really contains the target architecture.
    invariants = {
        root / "tools/ladybird/browser-upstream.sh": [
            "prepare-full-browser-host.py",
            "BouchaudBrowserHost",
            "Compositor",
            "WebWorker",
        ],
        root / "run.ps1": [
            "BOUCHAUD_BROWSER_HOST",
            '"BouchaudBrowserHost"',
            '"Compositor"',
            '"WebWorker"',
        ],
        root / ".github/workflows/ladybird-native-browser.yml": [
            "native-browser-bouchaud/BouchaudBrowserHost",
            "native-browser-bouchaud/Compositor",
            "native-browser-bouchaud/WebWorker",
        ],
    }

    for path, needles in invariants.items():
        data = planned[path]
        for needle in needles:
            if needle not in data:
                fail(f"preflight: invariant {needle!r} absent du resultat simule de {path.relative_to(root)}")

    print("Preflight Browser Host: OK")
    print("Checkout compatible avec PR #202.")
    print("Aucun fichier n'a ete modifie.")
    print("Tu peux maintenant lancer .\\APPLY-BROWSER-HOST.ps1 sans -VerifyOnly.")

def verify(root: Path) -> None:
    required = [
        root / "tools/ladybird/prepare-full-browser-host.py",
        root / "tools/ladybird/browser-upstream.sh",
        root / "run.ps1",
        root / ".github/workflows/ladybird-native-browser.yml",
    ]
    for path in required:
        if not path.is_file():
            fail(f"{path.relative_to(root)} absent")

    checks = {
        root / "tools/ladybird/browser-upstream.sh": [
            "prepare-full-browser-host.py", "BouchaudBrowserHost", "WebWorker", "Compositor"
        ],
        root / "run.ps1": [
            "BOUCHAUD_BROWSER_HOST", '"BouchaudBrowserHost"', '"Compositor"', '"WebWorker"'
        ],
        root / ".github/workflows/ladybird-native-browser.yml": [
            "native-browser-bouchaud/BouchaudBrowserHost",
            "native-browser-bouchaud/Compositor",
            "native-browser-bouchaud/WebWorker",
        ],
    }
    for path, needles in checks.items():
        data = path.read_text()
        for needle in needles:
            if needle not in data:
                fail(f"verification: {needle!r} absent de {path.relative_to(root)}")

    print("Verification statique Browser Host: OK")
    print("Validation restante: compilation Ladybird + QEMU runtime.")


def apply(root: Path) -> None:
    files = {
        root / "tools/ladybird/browser-upstream.sh": patch_browser_upstream,
        root / "run.ps1": patch_run_ps1,
        root / ".github/workflows/ladybird-native-browser.yml": patch_workflow,
    }

    for path in files:
        if not path.is_file():
            fail(f"{path.relative_to(root)} absent")

    planned = {path: patcher(path) for path, patcher in files.items()}

    backup = root / (".bouchaud-browser-host-full-backup-" + datetime.now().strftime("%Y%m%d-%H%M%S"))
    backup.mkdir(parents=True, exist_ok=False)

    for path in files:
        relative = path.relative_to(root)
        destination = backup / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, destination)

    for path, content in planned.items():
        path.write_text(content, newline="\n")

    print("Patch applique.")
    print("Sauvegarde:", backup)
    print("Aucun staging/commit/reset/clean Git n'a ete effectue.")
    verify(root)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", required=True)
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--apply", action="store_true")
    group.add_argument("--check", action="store_true")
    group.add_argument("--preflight", action="store_true")
    args = parser.parse_args()

    root = Path(args.root).resolve()
    if not (root / "Cargo.toml").is_file():
        fail("racine bouchaud-os invalide")

    if args.apply:
        apply(root)
    elif args.preflight:
        preflight(root)
    else:
        verify(root)


if __name__ == "__main__":
    main()
