from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import fabric_merge_gate


class MergeGate(unittest.TestCase):
    def test_rejects_absent_skipped_duplicate_and_stale_proof(self):
        exact = {
            "head_sha": "candidate", "head_branch": "topic", "event": "workflow_dispatch",
            "path": ".github/workflows/integration.yml", "status": "completed",
            "conclusion": "success", "run_number": 4, "run_attempt": 1, "id": 12,
        }
        success = {"name": "integration-gate", "status": "completed", "conclusion": "success"}
        skipped = {**success, "conclusion": "skipped"}
        for runs, jobs, expected in (
            ([], [], "MISSING"),
            ([{**exact, "head_sha": "stale"}], [success], "MISSING"),
            ([{**exact, "event": "pull_request"}], [success], "MISSING"),
            ([exact], [], "FAIL"),
            ([exact], [skipped], "FAIL"),
            ([exact], [success, success], "FAIL"),
            ([exact, {**exact, "run_number": 5, "status": "queued"}], [success], "FAIL"),
            ([exact], [success], "PASS"),
        ):
            with self.subTest(runs=runs, jobs=jobs), patch.object(
                fabric_merge_gate, "api", side_effect=[runs, jobs],
            ):
                report = fabric_merge_gate.evidence(
                    "owner/repo", "candidate", "topic", "integration.yml", ["integration-gate"],
                )
                self.assertEqual(report["status"], expected)
