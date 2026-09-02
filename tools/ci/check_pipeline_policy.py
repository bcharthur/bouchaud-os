#!/usr/bin/env python3
"""Structural CI contract for Bouchaud OS.

The checker intentionally validates properties, not exact line-for-line YAML.
It prevents a green pipeline from silently becoming a skipped pipeline again.
"""
from pathlib import Path
import sys

root = Path(".github/workflows")
errors: list[str] = []

def need(path: Path, needles: tuple[str, ...]):
    if not path.is_file():
        errors.append(f"{path}: absent")
        return ""
    text = path.read_text(encoding="utf-8")
    for needle in needles:
        if needle not in text:
            errors.append(f"{path}: contrat absent: {needle}")
    return text

manual_only = {
    "system-health.yml",
    "os-primitives.yml",
    "mm-ng6-smp4.yml",
    "ladybird-browser-host.yml",
    "ladybird-platform-smp4.yml",
    "ladybird-native-browser-v16.yml",
}
for name in manual_only:
    path = root / name
    if not path.is_file():
        errors.append(f"{name}: workflow attendu absent")
        continue
    text = path.read_text(encoding="utf-8")
    if "\n  push:" in text or "\n  pull_request:" in text or "\n  schedule:" in text:
        errors.append(f"{name}: doit rester manuel")

canon = need(
    root / "ladybird-native-browser.yml",
    ("name: ladybird-native-browser", "bouchaud-ladybird-native-browser"),
)

fast = need(root / "ci.yml", ("name: CI Fast", "fast-gate", "run_architecture_guards.sh", "run_host_tests.sh"))
integration = need(
    root / "integration.yml",
    ("name: Integration", "integration-gate", "run_qemu_smoke.sh", "run_mm_ng6.sh"),
)
nightly = need(root / "nightly.yml", ("name: Nightly Full", "nightly-gate"))

reliability = need(
    root / "reliability-v3.yml",
    (
        "name: Reliability V3",
        "reliability-gate",
        "matrix:",
        "cpu: [1, 2, 4, 8]",
        "qemu_matrix.py",
        "test_rendezvous_property.rs",
        "check_budgets.py",
    ),
)
release = need(
    root / "release.yml",
    (
        "name: Release",
        "attest-build-provenance@v2",
        "id-token: write",
        "attestations: write",
        "release_manifest.py",
        "reproducibility.py",
    ),
)
soak = need(
    root / "soak.yml",
    (
        "name: Long Soak",
        "bouchaud-soak",
        "8h",
        "24h",
        "72h",
        "soak.py",
    ),
)

if "continue-on-error: true" in reliability:
    errors.append("reliability-v3.yml: aucun test de fiabilite ne doit etre informatif")
if "continue-on-error: true" in release:
    errors.append("release.yml: une publication ne peut ignorer un echec")

protection = Path("tools/ci/configure_protection.ps1")
need(
    protection,
    ("fast-gate", "integration-gate", "reliability-gate", "branches/main/protection"),
)

for required in (
    Path("tools/ci/reliability/logscan.py"),
    Path("tools/ci/reliability/qemu_matrix.py"),
    Path("tools/ci/reliability/soak.py"),
    Path("tools/ci/reliability/powercut.py"),
    Path("tools/ci/reliability/reproducibility.py"),
    Path("tools/ci/budgets/reliability.json"),
):
    if not required.is_file():
        errors.append(f"{required}: composant reliability absent")

if errors:
    print("\n".join("ECHEC: " + e for e in errors), file=sys.stderr)
    raise SystemExit(1)

print("CI_POLICY_OK")
