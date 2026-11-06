#!/usr/bin/env python3
"""Non-regression contract for Bouchaud OS.

Fail-closed on deleted tests, weakened CI controls, missing historical safeguards
and stale HEAD. This gate is STRUCTURAL; runtime/P13/PHYSICAL evidence remains
separate and cannot be inferred from a PASS_STATIC verdict.
"""
from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Callable

SHA = re.compile(r"^[0-9a-f]{40}$")
TEST_NAME = re.compile(r"(^|/)test_[^/]+\.(py|rs|cpp|c)$")
RUST_TEST = re.compile(r"(?m)^\s*(?:#\[test\]|def test_[a-zA-Z0-9_]+\s*\(|TEST(?:_F|_P)?\s*\()")
NO_BUDGET_CHANGES = (
    "tools/ci/budgets/", "tools/ci/check_budgets.py",
    "tools/ci/reliability/budgets_endurance.py",
)
STRICT_WORKFLOWS = {
    ".github/workflows/ci.yml",
    ".github/workflows/reliability-v3.yml",
    ".github/workflows/p13-primitives-noyau.yml",
    ".github/workflows/release.yml",
}
CHECKED_WORKFLOWS = {
    ".github/workflows/ci.yml": ("name: CI Fast", "fast-gate", "run_architecture_guards.sh"),
    ".github/workflows/integration.yml": ("name: Integration", "integration-gate"),
    ".github/workflows/reliability-v3.yml": ("name: Reliability V3", "reliability-gate"),
    ".github/workflows/native-ipc-runtime.yml": (
        "name: Native IPC Runtime", "native-ipc / verdict",
        "NATIVE_IPC_VERDICT_OK", "if: always()",
    ),
    ".github/workflows/non-regression.yml": (
        "name: Non-regression Gate", "non-regression-gate",
        "pull_request:", "push:", "fetch-depth: 0",
        "non_regression.py", "test_non_regression.py",
    ),
}
RISK_DOMAINS = (
    ("usb_ab", ("src/drivers/usb/", "src/kernel/update/", "tools/ci/run_ab_rollback.sh",
                "system/update-core/"), ("Native IPC Runtime", "Reliability V3", "Integration")),
    ("smp", ("src/kernel/scheduler/", "src/kernel/process/thread/", "src/arch/x86_64/smp",
             "tools/smp/"), ("Reliability V3", "p13-primitives-noyau")),
    ("gui", ("src/gui/", "userland/services/windowd/", "userland/services/composited/",
             "tools/gui/"), ("CI Fast", "Integration")),
    ("storage", ("src/fs/", "tools/fs/", "userland/apps/notes/"),
     ("CI Fast", "Integration")),
    ("native_ipc", ("src/kernel/native/", "userland/services/", "userland/libs/",
                    "tools/native/"), ("Native IPC Runtime",)),
    ("physical", ("tools/release/", "src/boot/", "tools/reference/uefi-"),
     ("Integration", "Native IPC Runtime")),
    ("security", ("src/kernel/security/", "tools/security/"),
     ("CI Fast", "Reliability V3")),
    ("browser", ("tools/ladybird/", "tools/userland/navigateur/", "third_party/"),
     ("CI Fast", "ladybird-native-browser")),
    ("memory", ("src/kernel/memory/", "tools/mm/", "tools/memory/"),
     ("CI Fast", "Reliability V3", "Integration")),
    ("network", ("src/net/", "tools/net/", "src/drivers/net/"),
     ("CI Fast", "Integration")),
    ("product", ("src/gui/atelier/", "tools/product/", "userland/apps/"),
     ("CI Fast", "Integration")),
    ("governance", (".github/workflows/", "tools/ci/configure", "tools/ci/non_regression.py"),
     ("Non-regression Gate", "CI Fast")),
)


def read_tree(root: Path) -> Callable[[str], str | None]:
    def reader(path: str) -> str | None:
        file = root / path
        if not file.is_file() or file.is_symlink():
            return None
        return file.read_text(encoding="utf-8", errors="replace")
    return reader


def validate_contracts(read: Callable[[str], str | None]) -> list[str]:
    errors: list[str] = []

    def require(path: str, markers: tuple[str, ...]) -> str:
        body = read(path)
        if body is None:
            errors.append(f"CRITICAL_MISSING:{path}")
            return ""
        for marker in markers:
            if marker not in body:
                errors.append(f"CONTRACT_REMOVED:{path}:{marker}")
        return body

    ab = require("tools/ci/run_ab_rollback.sh", (
        "diagnostic_echec() {", "diagnostic_echec; rm -rf",
        "attend_etat_apres_commit() {", "SERVICED_UPDATE_STATE",
        'ATTENDU="B+ B="', 'ATTENDU="B- B- B- A= A="',
        "etat_apres_commit",
    ))
    start = ab.find('if [ -n "$slot" ] && grep -aq "result=committed"')
    stop = ab.find('if [ -n "$slot" ] && grep -aq "result=already_good"', start + 1)
    if start < 0 or stop < 0:
        errors.append("AB_COMMIT_BRANCH_MISSING")
    else:
        segment = ab[start:stop]
        if not ("attend_etat_apres_commit" in segment
                and 'verdict="$slot+"' in segment
                and segment.index("attend_etat_apres_commit") < segment.index('verdict="$slot+"')):
            errors.append("AB_POSTCOMMIT_READBACK_WEAKENED")

    require("src/gui/atelier/mod.rs", ("pub mod session;",))
    require("src/gui/atelier/session.rs", ("EchapSansMenu::Ignorer", "pub const fn action_echap"))
    require("tools/gui/test_session.rs", ("echap_bureau_vide_ne_quitte_pas",))
    wm = require("src/gui/window_manager.rs", (
        "action_echap(prise, !wins.is_empty())", "EchapSansMenu::Ignorer",
    ))
    start = wm.find("match action_echap(prise, !wins.is_empty())")
    stop = wm.find("\n                continue;", start)
    if start < 0 or stop < 0 or "quit = true" in wm[start:stop]:
        errors.append("ATELIER_ESCAPE_CAN_EXIT_SESSION")

    require("src/drivers/usb/xhci_active.rs", (
        "BOUCHAUD_USB_BULK_TRB_PERIME", "Some(trb_phys)",
    ))
    require("tools/platform/test_usb_bulk_trb_correlation.rs", (
        "BOUCHAUD_USB_BULK_TRB_PERIME",))
    native = require("tools/ci/run_native_services.sh", (
        "CALCULATRICE_RING3_OK", "SERVICED_QUALIFY_OK",
        "LOGD_QUALIFY_OK", "NATIVE_IPC_ATTRIBUTION",
    ))
    if "WINDOWD_RING3_IPC_OK" in native:
        require("userland/services/windowd/windowd.c", ("wd_apply(",))
        require("userland/tests/windowd-process-probe.c", ("WINDOWD_RING3_IPC_OK",))

    for path, markers in CHECKED_WORKFLOWS.items():
        body = require(path, markers)
        if path.endswith("non-regression.yml") and body:
            # This guard must run on *every* PR, including docs-only edits.
            segment = body.split("jobs:", 1)[0]
            if re.search(r"(?m)^  pull_request:\s*\n(?:\s+[^\n]*\n)*?\s+paths:", segment):
                errors.append("REGRESSION_WORKFLOW_PATH_FILTERED")
            if "continue-on-error: true" in body:
                errors.append("REGRESSION_WORKFLOW_PERMISSIVE")
    return errors


def git(root: Path, *args: str) -> str:
    result = subprocess.run(["git", "-C", str(root), *args], capture_output=True,
                            text=True, encoding="utf-8", errors="replace")
    if result.returncode:
        raise ValueError(f"git {' '.join(args[:2])}: {result.stderr.strip()[:250]}")
    return result.stdout.strip()


def changes_between(root: Path, base: str, head: str) -> list[tuple[str, str]]:
    if not SHA.fullmatch(base) or not SHA.fullmatch(head):
        raise ValueError("exact 40-hex base/head required")
    if git(root, "rev-parse", "HEAD") != head:
        raise ValueError("working tree HEAD differs from exact proposed SHA")
    common = git(root, "merge-base", base, head)
    raw = subprocess.run(
        ["git", "-C", str(root), "diff", "--name-status", "--no-renames", "-z",
         common, head],
        capture_output=True,
    )
    if raw.returncode:
        raise ValueError("git diff unavailable; cannot certify changes")
    parts = raw.stdout.decode("utf-8", errors="replace").split("\0")
    tokens = [item for item in parts if item]
    if len(tokens) % 2:
        raise ValueError("invalid git name-status stream")
    return [(tokens[i], tokens[i + 1]) for i in range(0, len(tokens), 2)]


def validate_changes(changes: list[tuple[str, str]],
                     before: Callable[[str], str | None],
                     after: Callable[[str], str | None]) -> list[str]:
    errors: list[str] = []
    for status, path in changes:
        kind = status[0]
        if kind == "D" and (
            TEST_NAME.search(path) or path.startswith("tools/verifie-")
            or path.startswith("tools/ci/reliability/")
            or path in CHECKED_WORKFLOWS
            or path in ("tools/ci/non_regression.py", "tools/ci/run_ab_rollback.sh")
        ):
            errors.append(f"PROTECTED_TEST_OR_GUARD_DELETED:{path}")
        if kind in ("A", "M", "D") and any(
            path == p or path.startswith(p) for p in NO_BUDGET_CHANGES
        ):
            errors.append(f"CI_THRESHOLD_CHANGE_REQUIRES_SEPARATE_REVIEW:{path}")
        if kind == "M" and TEST_NAME.search(path):
            old = before(path)
            new = after(path)
            if old is None or new is None:
                errors.append(f"TEST_COMPARISON_UNAVAILABLE:{path}")
            elif len(RUST_TEST.findall(new)) < len(RUST_TEST.findall(old)):
                errors.append(f"TEST_CASES_DECREASED:{path}")
        if kind in ("A", "M") and path in STRICT_WORKFLOWS:
            old = before(path) or ""
            new = after(path) or ""
            if new.count("continue-on-error: true") > old.count("continue-on-error: true"):
                errors.append(f"CI_GATE_WEAKENED:{path}")
    return errors


def required_evidence(paths: list[str]) -> dict[str, list[str]]:
    return {
        name: list(workflows)
        for name, prefixes, workflows in RISK_DOMAINS
        if any(any(path.startswith(prefix) for prefix in prefixes) for path in paths)
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path("."))
    parser.add_argument("--base", help="exact PR base SHA; omit only for tree-only check")
    parser.add_argument("--head", help="exact HEAD SHA; required with --base")
    parser.add_argument("--report", type=Path, default=Path("target/non-regression.json"))
    args = parser.parse_args()
    root = args.root.resolve()
    errors = validate_contracts(read_tree(root))
    paths: list[str] = []
    try:
        if (args.base is None) != (args.head is None):
            raise ValueError("--base and --head must be specified together")
        if args.base:
            changes = changes_between(root, args.base, args.head)
            paths = [path for _, path in changes]
            def before(path: str) -> str | None:
                cmd = subprocess.run(["git", "-C", str(root), "show",
                                      f"{args.base}:{path}"], capture_output=True,
                                     text=True, encoding="utf-8", errors="replace")
                return cmd.stdout if cmd.returncode == 0 else None
            errors += validate_changes(changes, before, read_tree(root))
    except ValueError as exc:
        errors.append(f"PROOF_UNAVAILABLE:{exc}")
    verdict = "BLOCK" if errors else "PASS_STATIC"
    report = {
        "schema": "bouchaud-non-regression/v1",
        "head": args.head or "TREE_ONLY",
        "base": args.base,
        "verdict": verdict,
        "blockers": sorted(set(errors)),
        "files_changed": paths,
        "required_runtime_evidence": required_evidence(paths),
        "scope": "structural guards only; not proof of CI runtime, P13 or Trigkey physical",
    }
    output = root / args.report
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n",
                      encoding="utf-8")
    print(f"NON_REGRESSION {verdict} files={len(paths)} errors={len(report['blockers'])}")
    for error in report["blockers"]:
        print(f"NON_REGRESSION_BLOCKER {error}", file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    raise SystemExit(main())
