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
    ".github/workflows/release-trigkey.yml",
    "tools/release/build_trigkey_image.sh",
    "tools/release/build_trigkey_ab_image.sh",
]
for rel in SHIPPING:
    p = ROOT / rel
    if p.exists():
        t = p.read_text(encoding="utf-8", errors="replace")
        for needle in ("banc-cle-dev", "BOUCHAUD_CLE_VERSION_DEV", "--cle-dev"):
            if needle in t:
                errors.append(f"{rel}: chemin de livraison qui mentionne {needle}")

# BOUCHAUD_RELEASE_V1: the signing workflow is MANUAL only -- a push (even
# to main) or a pull request must never reach the production secret.
rel = ROOT / ".github/workflows/release-trigkey.yml"
if not rel.exists():
    errors.append("release-trigkey.yml absent")
else:
    # Text parse (no PyYAML on every runner): top-level `on:` keys, then jobs.
    lignes = rel.read_text(encoding="utf-8").splitlines()
    def bloc(cle):
        i = next((k for k, l in enumerate(lignes) if l.rstrip() == f"{cle}:"), None)
        if i is None:
            return []
        fin = next((k for k in range(i + 1, len(lignes)) if lignes[k][:1] not in ("", " ", "#")), len(lignes))
        return lignes[i + 1:fin]
    declencheurs = {l.strip().rstrip(":") for l in bloc("on") if re.match(r"^  [A-Za-z_]", l)}
    if declencheurs != {"workflow_dispatch"}:
        errors.append(f"release-trigkey.yml: declencheurs {sorted(declencheurs)} (workflow_dispatch seul)")
    jobs, courant = {}, None
    for l in bloc("jobs"):
        m = re.match(r"^  ([A-Za-z0-9_-]+):\s*$", l)
        if m:
            courant = m.group(1)
            jobs[courant] = []
        elif courant:
            jobs[courant].append(l)
    for nom, corps in jobs.items():
        texte = "\n".join(corps)
        env = re.search(r"^    environment:\s*(\S+)", texte, re.M)
        if "secrets." in texte and (not env or env.group(1) not in ("trigkey-testing", "trigkey-stable")):
            errors.append(f"release-trigkey.yml: job {nom} lit un secret hors environnement protege")
        if "self-hosted" in texte:
            errors.append(f"release-trigkey.yml: job {nom} sur un runner non maitrise")
    if not jobs:
        errors.append("release-trigkey.yml: aucun job lu")

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
