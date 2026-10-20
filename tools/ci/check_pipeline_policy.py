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

# BOUCHAUD_CI_UN_BUILD_PAR_SHA_V1 : le run PR saute ce que le run push du
# MEME arbre teste deja (tools/ci/dedoublonne_pr.py). Deux regles :
#  * jamais sur un workflow dont un gate est un check requis de `main` -- un
#    job saute y compterait comme reussi ;
#  * ailleurs, AUCUN job racine hors du dedoublonnage : un job sans `needs`
#    tournerait deux fois, et un gate `always()` qui ne lit pas la decision
#    rendrait vert un run dont tout a ete saute.
def jobs_de(texte: str) -> dict[str, list[str]]:
    blocs: dict[str, list[str]] = {}
    courant = None
    dans_jobs = False
    for ligne in texte.split("\n"):
        if ligne == "jobs:":
            dans_jobs = True
            continue
        if not dans_jobs:
            continue
        if ligne and not ligne.startswith(" ") and not ligne.startswith("#"):
            break
        if ligne.startswith("  ") and not ligne.startswith("   ") and ligne.rstrip().endswith(":") and not ligne.strip().startswith("#"):
            courant = ligne.strip()[:-1]
            blocs[courant] = []
        elif courant:
            blocs[courant].append(ligne)
    return blocs

for name in ("ci.yml", "integration.yml", "reliability-v3.yml"):
    if "dedoublonne" in (root / name).read_text(encoding="utf-8"):
        errors.append(f"{name}: porte un gate requis -- pas de dedoublonnage push/PR")
for name in ("ladybird-native-browser.yml", "native-ipc-runtime.yml"):
    texte = (root / name).read_text(encoding="utf-8")
    blocs = jobs_de(texte)
    if "dedoublonne" not in blocs or "python3 tools/ci/dedoublonne_pr.py" not in "\n".join(blocs["dedoublonne"]):
        errors.append(f"{name}: job dedoublonne absent")
        continue
    for job, bloc in blocs.items():
        if job == "dedoublonne":
            continue
        corps = "\n".join(bloc)
        besoins = [l for l in bloc if l.startswith("    needs:")]
        conditions = [l for l in bloc if l.startswith("    if:")]
        if not besoins:
            errors.append(f"{name}: job racine {job} hors dedoublonnage")
        elif "dedoublonne" in besoins[0]:
            if not conditions or "needs.dedoublonne.outputs.doublon != 'true'" not in conditions[0]:
                errors.append(f"{name}: {job} depend de dedoublonne sans lire sa decision")
        if any("always()" in c or "!cancelled()" in c for c in conditions) and "dedoublonne" not in (besoins[0] if besoins else ""):
            errors.append(f"{name}: {job} tourne malgre un saut sans lire la decision")

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
