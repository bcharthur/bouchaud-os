#!/usr/bin/env python3
"""Classe les fichiers modifies pour piloter la CI Bouchaud OS.

Le script n'utilise aucune action tierce. Il fonctionne pour pull_request,
push et workflow_dispatch. Les sorties sont ecrites dans GITHUB_OUTPUT.
"""
from __future__ import annotations

import argparse
import os
import subprocess
from pathlib import Path

ZERO = "0" * 40


def git(*args: str) -> str:
    return subprocess.check_output(["git", *args], text=True).strip()


def changed_files(base: str | None, head: str) -> list[str]:
    if not base or base == ZERO:
        try:
            base = git("rev-parse", f"{head}^")
        except subprocess.CalledProcessError:
            return [p for p in git("ls-files").splitlines() if p]
    try:
        out = git("diff", "--name-only", f"{base}...{head}")
    except subprocess.CalledProcessError:
        out = git("diff", "--name-only", base, head)
    return [p for p in out.splitlines() if p]


def any_match(paths: list[str], prefixes: tuple[str, ...] = (), exact: tuple[str, ...] = (), suffixes: tuple[str, ...] = ()) -> bool:
    for path in paths:
        if path in exact:
            return True
        if prefixes and path.startswith(prefixes):
            return True
        if suffixes and path.endswith(suffixes):
            return True
    return False


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--base", default="")
    ap.add_argument("--head", default="HEAD")
    ap.add_argument("--all", action="store_true", help="force toutes les categories")
    args = ap.parse_args()

    paths = changed_files(args.base or None, args.head)
    forced = args.all or not paths

    workflow = forced or any_match(paths, prefixes=(".github/", "tools/ci/"))
    kernel = forced or any_match(
        paths,
        prefixes=(
            "src/", "targets/", ".cargo/", "tools/gui/", "tools/smp/",
            "tools/exec/", "tools/net/",
        ),
        exact=("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "x86_64-bouchaud_os.json"),
    )
    browser_engine = forced or any_match(
        paths,
        prefixes=("tools/userland/navigateur/",),
        exact=("tools/userland/test-moteur.sh", "tools/userland/build-quickjs.sh", "tools/userland/verifie-hote.sh"),
    )
    ladybird = forced or any_match(paths, prefixes=("tools/ladybird/",), exact=("third_party/UPSTREAM.md",))
    userland = forced or any_match(
        paths,
        prefixes=("tools/userland/", "userland/", "assets/", "fonts/"),
    )
    windows = forced or any_match(paths, suffixes=(".ps1", ".cmd", ".bat"))
    health = forced or any_match(paths, prefixes=("tools/health/",))
    mm = forced or any_match(
        paths,
        prefixes=("src/kernel/memory/", "src/kernel/process/", "src/kernel/scheduler/"),
        exact=("src/fs/backing.rs", "src/drivers/ata.rs", "tools/userland/mmstress.c", "tools/userland/build-mmstress.sh"),
    )
    primitives = forced or any_match(
        paths,
        prefixes=("src/compat/linux/", "src/kernel/abi/", "src/fs/"),
        exact=("src/drivers/ata.rs",),
    ) or any("probe" in Path(p).name for p in paths if p.startswith("tools/userland/"))
    docs_only = bool(paths) and all(p.startswith(("docs/", ".github/ISSUE_TEMPLATE/")) or p in {"README.md", "STATUS.md"} for p in paths)

    # Les workflows eux-memes doivent pouvoir prouver leur nouvelle configuration.
    if workflow:
        kernel = browser_engine = windows = True
        health = mm = primitives = True

    outputs = {
        "workflow": workflow,
        "kernel": kernel,
        "browser_engine": browser_engine,
        "ladybird": ladybird,
        "userland": userland,
        "windows": windows,
        "health": health,
        "mm": mm,
        "primitives": primitives,
        "docs_only": docs_only,
        "any_runtime": kernel or browser_engine or ladybird or userland or health or mm or primitives,
    }

    print("Fichiers modifies:")
    for path in paths:
        print(f"  - {path}")
    print("\nClassification:")
    for key, value in outputs.items():
        print(f"  {key:16} = {str(value).lower()}")

    output_path = os.environ.get("GITHUB_OUTPUT")
    if output_path:
        with open(output_path, "a", encoding="utf-8") as f:
            for key, value in outputs.items():
                f.write(f"{key}={str(value).lower()}\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
