#!/usr/bin/env python3
"""Structural CI contract for Bouchaud OS.

The checker intentionally validates properties, not exact line-for-line YAML.
It prevents a green pipeline from silently becoming a skipped pipeline again.
"""
from pathlib import Path
from fabric_coverage import p13_errors
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

# Only the reusable scheduler/primitives qualification is a mandatory standalone
# manual workflow. Historical debug standalones were removed: equivalent checks
# run in Integration, Ladybird native, Nightly or Reliability V3.
manual_only = {
    "os-primitives.yml",
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

fast = need(root / "ci.yml", ("name: CI Fast", "fast-gate", "run_architecture_guards.sh", "run_host_tests.sh",
                              "check_budgets.py", "check_pipeline_policy.py"))
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
for name in ("ladybird-native-browser.yml",):
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
_primitives_jobs = jobs_de((root / "os-primitives.yml").read_text(encoding="utf-8"))
_memory_call = _primitives_jobs.pop("memory-fault", [])
if not any(line.strip() == "uses: ./.github/workflows/fabric-memory.yml" for line in _memory_call):
    errors.append("os-primitives.yml: appel Memory Fabric absent")
_primitives_jobs.update(jobs_de((root / "fabric-memory.yml").read_text(encoding="utf-8")))
for _job, _bloc in _primitives_jobs.items():
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

# La couverture historique reste le plancher du plan central.
_primitive = (root / "os-primitives.yml").read_text()
_attendus = set(re.findall(r"tools/[A-Za-z0-9_./-]+", _primitive))
_attendus.update(("tools/ci/run_os_primitives.sh", "tools/ci/run_sigchld_repetition.sh",
                  "tools/userland/sigchld-multifil-probe.c", ".github/workflows/os-primitives.yml"))
errors.extend(p13_errors((root / "bouchaud-ci.yml").read_text(),
                        Path("tools/ci/fabric_compat_paths.json").read_text(),
                        _primitive, _attendus))

# IPC: les deux suites doivent réussir même quand une dépendance est sautée.
import yaml as _fabric_yaml
_ipc = _fabric_yaml.load((root / "native-ipc-runtime.yml").read_text(), Loader=_fabric_yaml.BaseLoader)
_central = _fabric_yaml.load((root / "bouchaud-ci.yml").read_text(), Loader=_fabric_yaml.BaseLoader)["jobs"]
_ij = _ipc["jobs"]
_iv = _ij["native-ipc-verdict"]
_ir = "\n".join(step.get("run", "") for step in _iv["steps"])
if (_ipc["on"] != {"workflow_call": ""}
        or _central.get("ipc") != {"needs": "plan", "if": "needs.plan.outputs.ipc == 'true'",
                                  "uses": "./.github/workflows/native-ipc-runtime.yml"}
        or "ipc" not in _central["verdict"]["needs"]
        or _ij.get("ab-rollback") != {"uses": "./.github/workflows/system-update.yml"}
        or set(_iv["needs"]) != {"native-ipc-runtime", "ab-rollback"}
        or _iv["if"] != "always()"
        or '[ "$R_RUNTIME" = success ] && [ "$R_AB" = success ] || exit 1' not in _ir):
    errors.append("IPC: les preuves runtime et A/B doivent toutes deux alimenter le verdict")

# BOUCHAUD_RUNNER_EPINGLE_V1 : aucun job Linux sur une etiquette MOUVANTE.
# Stabilisation TEMPORAIRE : voir docs/ci/RUNNER_EPINGLE_UBUNTU_2404.md (preuve,
# cause noyau -- pas de XSAVE/XCR0 -- et resolution definitive).
# `ubuntu-latest` est passe a Ubuntu 26.04 `amd64v3` le 19/11 : glibc et musl
# statiques du runner y sont compilees pour x86-64-v3 (AVX2), que le CPU
# invite `qemu64` n'a pas et que le noyau n'active pas (pas de XSAVE/XCR0).
# Toutes les sondes ring 3 liees sur le runner mouraient en silence, sur main
# comme sur les PR. Changer d'image est une decision, pas un effet de bord.
for _wf in sorted(root.glob("*.yml")):
    for _n, _l in enumerate(_wf.read_text(encoding="utf-8").splitlines(), 1):
        if re.match(r"\s*runs-on:\s*ubuntu-latest\s*$", _l):
            errors.append(f"{_wf.name}:{_n}: runs-on ubuntu-latest (etiquette mouvante ; epingler ubuntu-24.04)")

# Automatic non-regression check: always triggered, never a docs-only skip.
regression = need(root / "non-regression.yml",
                  ("name: Non-regression Gate", "non-regression-gate",
                   "pull_request:", "push:", "test_non_regression.py"))
if not (Path("tools/ci/non_regression.py").is_file() and
        Path("tools/ci/reliability/test_non_regression.py").is_file()):
    errors.append("non-regression: script ou tests de mutation absents")
if "non-regression-gate" not in protection.read_text(encoding="utf-8"):
    errors.append("protection main: non-regression-gate non requis")
if "continue-on-error: true" in regression:
    errors.append("non-regression: echec ignore par le workflow")

# BOUCHAUD_CI_TROIS_NIVEAUX_V1 : chaque SHA de branche est verifie UNE fois par
# niveau. Proprietes lues dans les DECLENCHEURS (pas dans le texte, que les
# commentaires satisfont) :
#  A. ci.yml : push sur toutes les branches, sans filtre, et PAS de
#     `pull_request` (le push publie deja `fast-gate` sur la tete de la PR) ;
#  B. aucun workflow ne se declenche a la fois sur `pull_request` et sur le push
#     d'une branche de travail : le meme arbre etait teste deux fois ;
#  C. Integration EXECUTE ses suites sur une PR : plus de libelle
#     `ci:integration`, et seule une PR documentaire est exemptee, dite telle ;
#  D. dans CI Fast, P10 est construit AVANT le noyau par defaut : le garde de
#     profondeur de pile lit le dernier binaire, qui doit etre celui livre.
import yaml  # noqa: E402

def declencheurs(nom: str) -> dict:
    d = yaml.safe_load((root / nom).read_text(encoding="utf-8")) or {}
    on = d.get(True, d.get("on")) or {}
    if isinstance(on, str):
        on = {on: None}
    elif isinstance(on, list):
        on = {k: None for k in on}
    return on

def branches_push(on: dict) -> list[str] | None:
    push = on.get("push")
    if "push" not in on:
        return None
    if not isinstance(push, dict) or "branches" not in push:
        return ["**"] if not (isinstance(push, dict) and "tags" in push) else []
    return [str(b) for b in push["branches"]]

on_ci = declencheurs("ci.yml")
if "pull_request" in on_ci:
    errors.append("ci.yml: declencheur pull_request (doublon du push sur le meme SHA)")
push_ci = on_ci.get("push") or {}
if branches_push(on_ci) != ["**"] or any(k in push_ci for k in ("paths", "paths-ignore", "branches-ignore")):
    errors.append("ci.yml: le niveau A doit tourner sur le push de TOUTE branche, sans filtre")

for chemin in sorted(root.glob("*.yml")):
    on = declencheurs(chemin.name)
    if "pull_request" not in on:
        continue
    br = branches_push(on)
    if br is None:
        continue
    travail = [b for b in br if b != "main"]
    if travail:
        errors.append(f"{chemin.name}: pull_request ET push de branches de travail {travail} "
                      "(le meme arbre teste deux fois)")

# E. CI Fast classe le delta CUMULE d'une branche (base 0...0 -> merge-base),
#    jamais le seul dernier push : sinon un push documentaire saute les
#    controles noyau sur la tete de la PR (BOUCHAUD_CI_DELTA_CUMULE_V1).
_classe = "\n".join(e.get("run", "") for e in yaml.safe_load(fast)["jobs"]["changes"]["steps"])
if "refs/heads/main" not in _classe or "--base 0000000000000000000000000000000000000000" not in _classe:
    errors.append("ci.yml: une branche de travail doit etre classee sur son delta cumule (base 0...0)")
# F. Les tests hote de CI Fast ont un noyau construit et l'exigent.
_hote = yaml.safe_load(fast)["jobs"]["host-tests"]["steps"]
_noms_h = [e.get("name", "") for e in _hote]
_suite = next((e for e in _hote if e.get("name") == "Suites de tests hote"), {})
if "cargo build" not in "\n".join(e.get("run", "") for e in _hote[:_noms_h.index("Suites de tests hote")] if _suite) \
        or (_suite.get("env") or {}).get("BOUCHAUD_EXIGE_NOYAU") != "1":
    errors.append("ci.yml: tests hote sans noyau construit ou sans BOUCHAUD_EXIGE_NOYAU=1 (symbolisation sautee)")

if "ci:integration" in integration:
    errors.append("integration.yml: libelle ci:integration -- une PR doit executer les suites sans libelle")
if "INTEGRATION_GATE_DOCS_ONLY" not in integration:
    errors.append("integration.yml: l'exemption documentaire doit etre dite (INTEGRATION_GATE_DOCS_ONLY)")

_kf = yaml.safe_load(fast)["jobs"]["kernel-fast"]["steps"]
_noms = [e.get("name", "") for e in _kf]
try:
    if _noms.index("P10 GPU - compilation avec et sans compositeur VIRGL") > _noms.index("Compiler le noyau"):
        errors.append("ci.yml: P10 construit APRES le noyau par defaut -- le garde de pile lirait le binaire p10")
    if _noms.index("Garde-fous d'architecture") < _noms.index("Compiler le noyau"):
        errors.append("ci.yml: garde-fous lances avant la compilation du noyau analyse")
except ValueError as exc:
    errors.append(f"ci.yml: etape attendue absente de kernel-fast ({exc})")

if errors:
    print("\n".join("ECHEC: " + e for e in errors), file=sys.stderr)
    raise SystemExit(1)

print("CI_POLICY_OK")
