"""Mutation tests for the structural non-regression gate (offline)."""
from __future__ import annotations
import importlib.util
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[3]
PATH = ROOT / "tools/ci/non_regression.py"
SPEC = importlib.util.spec_from_file_location("non_regression", PATH)
assert SPEC is not None and SPEC.loader is not None
gate = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(gate)


class TestNonRegression(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.contents = {}
        # Utiliser l'arbre reel comme source : les gardes conditionnels peuvent
        # exiger de nouveaux fichiers sans mise a jour manuelle du fixture.
        cls.reader = gate.read_tree(ROOT)
        for path in (
            "tools/ci/run_ab_rollback.sh", "src/gui/atelier/mod.rs",
            "src/gui/atelier/session.rs", "tools/gui/test_session.rs",
            "src/gui/window_manager.rs", "src/drivers/usb/xhci_active.rs",
            "tools/platform/test_usb_bulk_trb_correlation.rs",
            "tools/ci/run_native_services.sh",
            "userland/services/windowd/windowd.c",
            "userland/tests/windowd-process-probe.c",
            *gate.CHECKED_WORKFLOWS,
        ):
            cls.contents[path] = cls.reader(path)

    def check(self, changed=None):
        # Les mutations explicites (y compris None = fichier supprime) priment
        # sur l'arbre reel. Pour les autres fichiers, ne jamais simuler
        # artificiellement leur absence a cause d'un fixture incomplet.
        changed = changed or {}
        def read(path):
            if path in changed:
                return changed[path]
            return self.reader(path)
        return gate.validate_contracts(read)

    def test_live_tree_has_no_missing_guards(self):
        self.assertEqual(self.check(), [])

    def test_windowd_fixture_contains_real_service_and_probe(self):
        # A conditional contract must be tested against the *complete* tree.
        self.assertIn("WINDOWD_RING3_IPC_OK",
                      self.contents["tools/ci/run_native_services.sh"])
        self.assertIn("wd_apply(",
                      self.contents["userland/services/windowd/windowd.c"])
        self.assertIn("WINDOWD_RING3_IPC_OK",
                      self.contents["userland/tests/windowd-process-probe.c"])
        self.assertEqual(self.check(), [])

    def test_windowd_service_deleted_is_blocked(self):
        errors = self.check({"userland/services/windowd/windowd.c": None})
        self.assertIn("CRITICAL_MISSING:userland/services/windowd/windowd.c", errors)

    def test_windowd_probe_deleted_is_blocked(self):
        errors = self.check({"userland/tests/windowd-process-probe.c": None})
        self.assertIn("CRITICAL_MISSING:userland/tests/windowd-process-probe.c", errors)

    def test_windowd_service_contract_removed_is_blocked(self):
        service = self.contents["userland/services/windowd/windowd.c"]
        errors = self.check({"userland/services/windowd/windowd.c":
                             service.replace("wd_apply(", "wd_applied(")})
        self.assertIn("CONTRACT_REMOVED:userland/services/windowd/windowd.c:wd_apply(", errors)

    def test_windowd_probe_contract_removed_is_blocked(self):
        probe = self.contents["userland/tests/windowd-process-probe.c"]
        errors = self.check({"userland/tests/windowd-process-probe.c":
                             probe.replace("WINDOWD_RING3_IPC_OK", "WINDOWD_RING3_IPC_BROKEN")})
        self.assertIn("CONTRACT_REMOVED:userland/tests/windowd-process-probe.c:WINDOWD_RING3_IPC_OK",
                      errors)

    def test_removing_postcommit_readback_fails(self):
        path = "tools/ci/run_ab_rollback.sh"
        bad = self.contents[path].replace("attend_etat_apres_commit || true", "")
        self.assertIn("AB_POSTCOMMIT_READBACK_WEAKENED", self.check({path: bad}))

    def test_removing_diagnostic_fails(self):
        path = "tools/ci/run_ab_rollback.sh"
        bad = self.contents[path].replace("diagnostic_echec() {", "")
        self.assertTrue(any("diagnostic_echec" in e for e in self.check({path: bad})))

    def test_escape_regression_fails(self):
        path = "src/gui/window_manager.rs"
        text = self.contents[path]
        start = text.index("match action_echap(prise, !wins.is_empty())")
        bad = text[:start] + text[start:].replace(
            "EchapSansMenu::Ignorer => {", "EchapSansMenu::Ignorer => { quit = true;", 1)
        self.assertIn("ATELIER_ESCAPE_CAN_EXIT_SESSION", self.check({path: bad}))

    def test_lost_usb_event_correlation_fails(self):
        path = "src/drivers/usb/xhci_active.rs"
        bad = self.contents[path].replace("Some(trb_phys)", "None")
        self.assertTrue(any("Some(trb_phys)" in e for e in self.check({path: bad})))

    def test_disabled_regression_job_is_rejected(self):
        path = ".github/workflows/non-regression.yml"
        bad = self.contents[path].replace(
            "    if: always()", "    if: false", 1)
        self.assertIn("REGRESSION_JOB_DISABLED", self.check({path: bad}))

    def test_echoed_guard_is_not_execution(self):
        path = ".github/workflows/non-regression.yml"
        bad = self.contents[path].replace(
            "python3 tools/ci/non_regression.py --base", "echo python3 tools/ci/non_regression.py --base"
        ).replace(
            "python3 tools/ci/non_regression.py\n", "echo python3 tools/ci/non_regression.py\n"
        )
        self.assertIn("REGRESSION_EXECUTABLE_STEP_MISSING:guard", self.check({path: bad}))

    def test_guard_failure_must_not_be_swallowed(self):
        path = ".github/workflows/non-regression.yml"
        bad = self.contents[path].replace(
            'python3 tools/ci/non_regression.py --base "$BASE_SHA" --head "$HEAD_SHA"',
            'python3 tools/ci/non_regression.py --base "$BASE_SHA" --head "$HEAD_SHA" || true',
        ).replace(
            "python3 tools/ci/non_regression.py\n",
            "python3 tools/ci/non_regression.py || true\n",
        )
        self.assertIn("REGRESSION_EXECUTABLE_STEP_MISSING:guard", self.check({path: bad}))

    def test_disabled_guard_step_is_rejected(self):
        path = ".github/workflows/non-regression.yml"
        bad = self.contents[path].replace(
            "      - name: Invariants critiques sur HEAD et diff avec la base exacte",
            "      - name: Invariants critiques sur HEAD et diff avec la base exacte\n        if: false",
        )
        self.assertIn("REGRESSION_EXECUTABLE_STEP_MISSING:guard", self.check({path: bad}))

    def test_disabled_mutation_suite_is_rejected(self):
        path = ".github/workflows/non-regression.yml"
        bad = self.contents[path].replace(
            "      - name: Eprouver les controles contre des mutations volontaires",
            "      - name: Eprouver les controles contre des mutations volontaires\n        if: false",
        )
        self.assertIn("REGRESSION_EXECUTABLE_STEP_MISSING:unittest", self.check({path: bad}))

    def test_no_workflow_cannot_pass(self):
        self.assertIn("CRITICAL_MISSING:.github/workflows/non-regression.yml",
                      self.check({".github/workflows/non-regression.yml": None}))

    def test_deleting_guard_or_test_is_forbidden(self):
        changes = [("D", "tools/gui/test_session.rs"),
                   ("D", "tools/verifie-ordre-verrous.py"),
                   ("D", "tools/ci/run_ab_rollback.sh")]
        faults = gate.validate_changes(changes, lambda p: "content", lambda p: None)
        self.assertEqual(sum("DELETED" in x for x in faults), 3)

    def test_existing_test_count_cannot_decrease(self):
        defects = gate.validate_changes(
            [("M", "tools/gui/test_session.rs")],
            lambda p: "#[test]\nfn first() {}\n#[test]\nfn second() {}",
            lambda p: "#[test]\nfn first() {}",
        )
        self.assertEqual(defects, ["TEST_CASES_DECREASED:tools/gui/test_session.rs"])

    def test_ci_threshold_change_is_blocked(self):
        faults = gate.validate_changes(
            [("M", "tools/ci/budgets/reliability.json")],
            lambda p: "{}", lambda p: "{}")
        self.assertTrue(any("CI_THRESHOLD_CHANGE" in x for x in faults))

    def test_ci_continue_on_error_never_added(self):
        faults = gate.validate_changes(
            [("M", ".github/workflows/reliability-v3.yml")],
            lambda p: "jobs:\n", lambda p: "jobs:\ncontinue-on-error: true\n",
        )
        self.assertTrue(any("CI_GATE_WEAKENED" in x for x in faults))

    def test_risk_mapping_no_automatic_green(self):
        risks = gate.required_evidence([
            "src/drivers/usb/xhci_active.rs",
            "src/kernel/process/thread/courant.rs",
            "userland/services/healthd/healthd.c",
        ])
        self.assertIn("Native IPC Runtime", risks["usb_ab"])
        self.assertIn("p13-primitives-noyau", risks["smp"])
        self.assertIn("native_ipc", risks)

    def test_all_twelve_chantiers_are_classified(self):
        examples = {
            "memory": "src/kernel/memory/heap.rs",
            "network": "src/net/internet/ipv4.rs",
            "product": "src/gui/atelier/mouvement.rs",
            "governance": ".github/workflows/ci.yml",
            "security": "src/kernel/security/wx.rs",
            "browser": "tools/ladybird/prepare-browser-runtime-link.py",
            "physical": "tools/release/build_trigkey_image.sh",
            "native_ipc": "src/kernel/native/time.rs",
            "storage": "src/fs/ramfs.rs",
            "gui": "src/gui/window_manager.rs",
            "smp": "src/kernel/process/thread/courant.rs",
            "usb_ab": "src/drivers/usb/xhci_active.rs",
        }
        for name, file in examples.items():
            with self.subTest(domain=name):
                self.assertIn(name, gate.required_evidence([file]))



if __name__ == "__main__":
    unittest.main()
