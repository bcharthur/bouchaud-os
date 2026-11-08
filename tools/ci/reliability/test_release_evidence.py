"""Automated host regression for fail-closed release evidence (no GitHub network)."""
from __future__ import annotations
import importlib.util
from pathlib import Path
import unittest

PATH = Path(__file__).resolve().parents[2] / "health" / "release_truth.py"
SPEC = importlib.util.spec_from_file_location("release_truth", PATH)
assert SPEC is not None and SPEC.loader is not None
release_truth = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release_truth)

SHA = "a" * 40
OLD_SHA = "b" * 40
NOW = "2026-11-06T17:00:00+00:00"


def sample() -> dict:
    names = (*release_truth.REQUIRED_PR_CHECKS, *release_truth.RELEASE_CHECKS)
    return {
        "main_sha": SHA,
        "main_protected": True,
        "run_list_truncated": False,
        "workflow_runs": [
            {"name": n, "head_sha": SHA, "event": "push", "status": "completed",
             "conclusion": "success", "created_at": "2026-11-06T14:00:00Z",
             "run_attempt": 1, "id": i}
            for i, n in enumerate(names, start=1)
        ],
        "issues": [],
        "snapshot": {
            "observed_main_head": SHA,
            "as_of": "2026-11-06",
            "updates_automatically": False,
            "baseline_status": "ESTABLISHED",
            "physical": "QUALIFIED",
            "p13": {"P13_FINAL_STRICT_OK": True},
        },
    }


class ReleaseTruthTests(unittest.TestCase):
    def test_only_full_evidence_can_be_reviewed(self):
        result = release_truth.evaluate(sample(), NOW)
        self.assertEqual(result["release_verdict"], "GO_FOR_REVIEW")
        self.assertEqual(result["blockers"], [])
        self.assertEqual(result["exact_main_sha"], SHA)

    def test_stale_previous_sha_never_qualifies(self):
        data = sample()
        data["snapshot"]["observed_main_head"] = OLD_SHA
        result = release_truth.evaluate(data, NOW)
        self.assertEqual(result["release_verdict"], "NO_GO")
        self.assertIn("SNAPSHOT:stale_sha", result["blockers"])

    def test_past_green_does_not_mask_current_failure(self):
        data = sample()
        data["workflow_runs"].append({
            "name": "Native IPC Runtime", "head_sha": SHA,
            "event": "push", "status": "completed", "conclusion": "failure",
            "created_at": "2026-11-06T16:00:00Z", "id": 100, "run_attempt": 1,
        })
        report = release_truth.evaluate(data, NOW)
        self.assertEqual(report["workflow_results"]["Native IPC Runtime"]["run_id"], 100)
        self.assertIn("CI:Native IPC Runtime=FAIL", report["blockers"])

    def test_other_sha_green_does_not_replace_missing_workflow(self):
        data = sample()
        for run in data["workflow_runs"]:
            if run["name"] == "Reliability V3":
                run["head_sha"] = OLD_SHA
        result = release_truth.evaluate(data, NOW)
        self.assertIn("CI:Reliability V3=MISSING", result["blockers"])

    def test_pending_not_passed(self):
        data = sample()
        data["workflow_runs"][0]["status"] = "in_progress"
        data["workflow_runs"][0]["conclusion"] = None
        report = release_truth.evaluate(data, NOW)
        self.assertIn("CI:CI Fast=PENDING", report["blockers"])

    def test_open_bug_and_unprotected_main_block_release(self):
        data = sample()
        data["main_protected"] = False
        data["issues"] = [{"number": 264, "title": "bug(p13): intermittent scheduler"}]
        result = release_truth.evaluate(data, NOW)
        self.assertIn("GOVERNANCE:main_unprotected", result["blockers"])
        self.assertIn("ISSUE:#264", result["blockers"])

    def test_physical_unproven_blocks_even_all_ci_green(self):
        data = sample()
        data["snapshot"]["physical"] = "NOT_STARTED"
        report = release_truth.evaluate(data, NOW)
        self.assertIn("PHYSICAL:not_qualified", report["blockers"])

    def test_incomplete_api_page_fails_closed(self):
        data = sample()
        data["run_list_truncated"] = True
        result = release_truth.evaluate(data, NOW)
        self.assertIn("EVIDENCE:workflow_list_truncated", result["blockers"])

    def test_bad_sha_rejected(self):
        data = sample()
        data["main_sha"] = "short"
        with self.assertRaises(ValueError):
            release_truth.evaluate(data, NOW)


if __name__ == "__main__":
    unittest.main()
