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

    def test_artifact_metadata_requires_the_run_attempt_and_digest(self):
        run = {"id": 12, "head_sha": "candidate", "run_started_at": "2026-11-26T18:00:00Z"}
        artifact = {
            "expired": False, "size_in_bytes": 20,
            "workflow_run": {"id": 12, "head_sha": "candidate"},
            "created_at": "2026-11-26T18:00:01Z", "digest": "sha256:" + "a" * 64,
        }
        self.assertTrue(fabric_merge_gate.valid_artifact(artifact, run))
        for change in (
            {"expired": True}, {"size_in_bytes": 0}, {"digest": None},
            {"workflow_run": {"id": 11, "head_sha": "candidate"}},
            {"workflow_run": {"id": 12, "head_sha": "stale"}},
            {"created_at": "2026-11-26T17:59:00Z"},
        ):
            with self.subTest(change=change):
                self.assertFalse(fabric_merge_gate.valid_artifact({**artifact, **change}, run))
