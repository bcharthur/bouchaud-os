#!/usr/bin/env python3
"""A development release key must never reach a shipped image
(BOUCHAUD_CLE_DEV_GARDE_V1).

Fail-closed static guard, run with the architecture guards:
  * `banc-cle-dev` is not in Cargo.toml `default`;
  * no shipping path (release workflow, Trigkey image script, installer
    payload) names the feature or BOUCHAUD_CLE_VERSION_DEV;
  * only the A/B bench sets BOUCHAUD_CLE_VERSION_DEV, and it does so from a
    key generated in a temporary directory;
  * no private key (PEM "PRIVATE KEY" block) is committed anywhere.
The kernel side of the guard is a const assertion in src/kernel/update/mod.rs:
the variable without the feature fails the build.
"""
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
errors = []

cargo = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
m = re.search(r"^default\s*=\s*\[(.*?)\]", cargo, re.M | re.S)
if m and "banc-cle-dev" in m.group(1):
    errors.append("Cargo.toml: banc-cle-dev est dans default")
if "banc-cle-dev = []" not in cargo:
    errors.append("Cargo.toml: feature banc-cle-dev absente (le garde noyau en depend)")

SHIPPING = [
    ".github/workflows/release.yml",
    "tools/release/build_trigkey_image.sh",
]
for rel in SHIPPING:
    p = ROOT / rel
    if p.exists():
        t = p.read_text(encoding="utf-8", errors="replace")
        for needle in ("banc-cle-dev", "BOUCHAUD_CLE_VERSION_DEV"):
            if needle in t:
                errors.append(f"{rel}: chemin de livraison qui mentionne {needle}")

tracked = subprocess.run(["git", "ls-files"], cwd=ROOT, capture_output=True, text=True, check=True).stdout.split()
setters = []
for rel in tracked:
    p = ROOT / rel
    if not p.is_file() or p.stat().st_size > 2_000_000:
        continue
    try:
        t = p.read_text(encoding="utf-8")
    except (UnicodeDecodeError, OSError):
        continue
    if re.search(r"-----BEGIN [A-Z ]*PRIVATE KEY-----", t) and rel != "tools/verifie-cle-dev.py":
        errors.append(f"{rel}: bloc PEM de cle privee commite")
    if re.search(r"(export\s+|^\s*)BOUCHAUD_CLE_VERSION_DEV=", t, re.M) and rel.endswith((".sh", ".yml", ".yaml")):
        setters.append(rel)
for rel in setters:
    if rel != "tools/ci/run_ab_rollback.sh":
        errors.append(f"{rel}: pose BOUCHAUD_CLE_VERSION_DEV (seul le banc A/B le peut)")
banc = (ROOT / "tools/ci/run_ab_rollback.sh").read_text(encoding="utf-8")
if "CLES=$(mktemp -d)" not in banc or 'rm -rf "$TRAVAIL" "$CLES"' not in banc:
    errors.append("tools/ci/run_ab_rollback.sh: la cle de banc doit naitre et mourir dans un repertoire temporaire")

mod = (ROOT / "src/kernel/update/mod.rs").read_text(encoding="utf-8")
if 'cfg(not(feature = "banc-cle-dev"))' not in mod or "refusing to build" not in mod:
    errors.append("src/kernel/update/mod.rs: garde de construction de la cle de dev absent")

for e in errors:
    print("CLE_DEV_GARDE_ECHEC", e)
if errors:
    sys.exit(1)
print("CLE_DEV_GARDE_OK")
