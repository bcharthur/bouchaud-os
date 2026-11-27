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

    def test_pr_profile_is_conservative(self):
        import fabric_pr_gate

        for paths, profile in (
            (b"docs/current/CI_CD_FABRIC.md\x00", "FAST"),
            (b"src/kernel/memory/vmm.rs\x00", "FABRIC"),
            (b".github/workflows/bouchaud-ci.yml\x00", "FULL"),
            (b"src/kernel/memory/vmm.rs\x00docs/tool.py\x00", "FULL"),
            (b"unmapped.bin\x00", "FULL"),
            (b"", "FULL"),
        ):
            with self.subTest(paths=paths), patch.object(
                fabric_pr_gate.subprocess, "check_output", return_value=paths,
            ):
                self.assertEqual(fabric_pr_gate.required_profile("base"), profile)

    def test_certification_waits_then_rejects_failed_prerequisites(self):
        import fabric_request_proofs

        pending = {"status": "in_progress", "conclusion": None}
        passed = {"status": "completed", "conclusion": "success"}
        with patch.object(fabric_request_proofs, "latest_run", side_effect=[pending, passed]), patch.object(
            fabric_request_proofs.time, "sleep",
        ) as delay:
            fabric_request_proofs.wait_for_prerequisites("owner/repo", "sha", "branch", ["nightly"])
            delay.assert_called_once()
        failed = {**passed, "id": 14, "conclusion": "failure"}
        with patch.object(fabric_request_proofs, "latest_run", return_value=failed):
            with self.assertRaisesRegex(SystemExit, "Prérequis en échec"):
                fabric_request_proofs.wait_for_prerequisites("owner/repo", "sha", "branch", ["nightly"])

    def test_certification_dispatches_dedicated_p13_after_wait(self):
        import json
        import fabric_request_proofs

        workflow = "p13-final-candidate.yml"
        def latest(repo, sha, branch, name):
            return None if name == workflow else {"status": "completed", "conclusion": "success"}

        with patch.dict(fabric_request_proofs.os.environ, {
            "GITHUB_REPOSITORY": "owner/repo", "GITHUB_SHA": "candidate", "FABRIC_PROOF_BRANCH": "topic",
        }), patch.object(sys, "argv", ["fabric_request_proofs.py", "CERTIFICATION"]), patch.object(
            fabric_request_proofs, "latest_run", side_effect=latest,
        ), patch.object(fabric_request_proofs, "wait_for_prerequisites") as wait, patch.object(
            fabric_request_proofs.subprocess, "check_output", return_value="candidate\n",
        ), patch.object(fabric_request_proofs.subprocess, "run") as dispatch:
            self.assertEqual(fabric_request_proofs.main(), 0)
            wait.assert_called_once_with(
                "owner/repo", "candidate", "topic", ["ladybird-native-browser.yml", "nightly.yml"],
            )
            dispatch.assert_called_once()
            self.assertEqual(dispatch.call_args.args[0][3], workflow)
            self.assertEqual(json.loads(dispatch.call_args.kwargs["input"])["perimetre"], "complet")
