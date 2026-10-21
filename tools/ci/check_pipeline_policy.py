#!/usr/bin/env python3
"""Structural CI contract for Bouchaud OS.

The checker intentionally validates properties, not exact line-for-line YAML.
It prevents a green pipeline from silently becoming a skipped pipeline again.
"""
from pathlib import Path
import re
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

# BOUCHAUD_CI_UN_BUILD_PAR_SHA_V2 : le run PR peut REFLETER le run push du meme
# arbre (tools/ci/dedoublonne_pr.py) au lieu de le refaire -- jamais en sautant
# son verdict. Regles :
#  * jamais sur un workflow dont un gate est un check requis de `main` ;
#  * aucun job racine hors du dedoublonnage, et toute racine lit `reutilise` ;
#  * UN job de verdict, `if: always()`, qui contient le reflet
#    (`dedoublonne_pr.py attend`) ET un verdict execute : il n'est jamais saute,
#    et un run push en echec, annule ou saute le rend rouge ;
#  * plus aucune lecture de la sortie V1 `doublon` (qui sautait le verdict).
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
    if "dedoublonne" not in blocs or "python3 tools/ci/dedoublonne_pr.py" not in "\n".join(blocs["dedoublonne"]) \
            or "reutilise:" not in "\n".join(blocs["dedoublonne"]):
        errors.append(f"{name}: job dedoublonne absent ou sans sortie `reutilise`")
        continue
    if re.search(r"outputs\.doublon\b", texte):
        errors.append(f"{name}: lit encore la sortie V1 `doublon` (verdict saute)")
    verdicts = [j for j, b in blocs.items() if "dedoublonne_pr.py attend" in "\n".join(b)]
    if len(verdicts) != 1:
        errors.append(f"{name}: il faut exactement un job de verdict qui reflete le run push ({verdicts})")
    for job, bloc in blocs.items():
        if job == "dedoublonne":
            continue
        corps = "\n".join(bloc)
        besoins = [l for l in bloc if l.startswith("    needs:")]
        conditions = [l for l in bloc if l.startswith("    if:")]
        if not besoins:
            errors.append(f"{name}: job racine {job} hors dedoublonnage")
            continue
        if job in verdicts:
            if not conditions or conditions[0].strip() != "if: always()":
                errors.append(f"{name}: le verdict {job} doit etre `if: always()` (jamais saute)")
            if "if: needs.dedoublonne.outputs.reutilise != 'true'" not in corps:
                errors.append(f"{name}: le verdict {job} n'a pas de verdict execute hors reutilisation")
            if "dedoublonne" not in besoins[0]:
                errors.append(f"{name}: le verdict {job} ne lit pas la decision")
            continue
        if "dedoublonne" in besoins[0]:
            if not conditions or "needs.dedoublonne.outputs.reutilise != 'true'" not in conditions[0]:
                errors.append(f"{name}: {job} depend de dedoublonne sans lire sa decision")
        if any("always()" in c or "!cancelled()" in c for c in conditions) and "dedoublonne" not in besoins[0]:
            errors.append(f"{name}: {job} tourne malgre un saut sans lire la decision")

# BOUCHAUD_PREUVE_INDEPENDANTE_V1 : dans os-primitives, une etape apres le
# noyau construit tourne meme si une etape precedente a echoue. Run 39503091750 :
# un temoin TCG rouge a fait sauter « Acces a /dev/kvm », et toute l'attribution
# P13 KVM (SMP1/2/4/8) est tombee en « /dev/kvm inaccessible » -- zero preuve.
# Le job reste rouge ; seules les preuves independantes sont conservees.
for _job, _bloc in jobs_de((root / "os-primitives.yml").read_text(encoding="utf-8")).items():
    _etapes, _courante = [], None
    for _l in _bloc:
        _m = re.match(r"^      - (?:name: (.*)|uses: .*|run: .*)$", _l)
        if _m:
            _courante = {"name": (_m.group(1) or "?").strip(), "lignes": []}
            _etapes.append(_courante)
        elif _courante is not None:
            _courante["lignes"].append(_l)
    _apres = False
    for _e in _etapes:
        _corps = "\n".join(_e["lignes"])
        if "id: bootimage" in _corps:
            _apres = True
            continue
        if not _apres:
            continue
        _si = next((l for l in _e["lignes"] if l.startswith("        if:")), "")
        if "!cancelled()" not in _si and "always()" not in _si:
            errors.append(f"os-primitives.yml: {_job} / {_e['name']} saute si une etape precedente echoue")
    if not _apres:
        errors.append(f"os-primitives.yml: {_job} sans etape `id: bootimage`")

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
