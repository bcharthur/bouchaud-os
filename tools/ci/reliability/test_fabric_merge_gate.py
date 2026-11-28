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

    def test_pr_gate_requires_live_attempts_and_archives(self):
        import fabric_pr_gate

        workflow = "integration.yml"
        run = {"id": 12, "run_attempt": 1, "head_sha": "candidate", "status": "completed",
               "conclusion": "success", "created_at": "2026-11-26T18:00:00Z"}
        proof = {"run": 12, "attempt": 1}
        for candidate, expected in (
            (run, None),
            ({**run, "run_attempt": 2}, ValueError),
            ({**run, "id": 13}, ValueError),
            ({**run, "conclusion": "cancelled"}, ValueError),
            ({**run, "conclusion": "failure"}, ValueError),
            ({**run, "status": "in_progress", "conclusion": None}, fabric_pr_gate.PendingProof),
            (None, ValueError),
        ):
            with self.subTest(candidate=candidate), patch.object(
                fabric_pr_gate, "latest_run", return_value=candidate,
            ), patch.object(fabric_pr_gate, "verify_archives") as archive:
                def verify():
                    return fabric_pr_gate.verify_independent_campaigns(
                        "owner/repo", "candidate", "topic", "api", [workflow], {workflow: proof},
                    )
                if expected:
                    with self.assertRaises(expected):
                        verify()
                    archive.assert_not_called()
                else:
                    verify()
                    archive.assert_called_once()

        digest = "sha256:" + "a" * 64
        attest = {"checked": {"journal": {"id": 42, "digest": digest}}}
        live = {"id": 42, "digest": digest, "name": "journal", "expired": False, "size_in_bytes": 20,
                "workflow_run": {"id": 12, "head_sha": "candidate"}, "created_at": "2026-11-26T18:00:00Z"}
        with patch.object(fabric_pr_gate, "api", return_value=[live]):
            fabric_pr_gate.verify_archives("api", run, attest, ["journal"])
        for artifacts in ([], [live, live], [{**live, "expired": True}], [{**live, "digest": "sha256:" + "b" * 64}]):
            with self.subTest(artifacts=artifacts), patch.object(fabric_pr_gate, "api", return_value=artifacts):
                with self.assertRaisesRegex(ValueError, "Archive attestée"):
                    fabric_pr_gate.verify_archives("api", run, attest, ["journal"])

    def test_full_report_identity_and_coverage(self):
        import hashlib
        import fabric_pr_gate

        head, branch = "candidate", "topic"
        run = {"head_sha": head, "head_branch": f"ci-fabric/FULL/{head}/{branch}", "event": "push",
               "path": ".github/workflows/bouchaud-ci.yml", "status": "completed",
               "conclusion": "success", "run_number": 1, "run_attempt": 2, "id": 12}
        required = fabric_pr_gate.PROFILES["FULL"]
        names = (*required["current"], *required["campaigns"], "current-archives")
        proofs = {key: {"status": "PASS"} for key in names}
        report = {"sha": head, "branch": branch, "profile": "FULL", "decision": "FULL_PASS",
                  "registry_sha256": hashlib.sha256(fabric_pr_gate.REGISTRY.read_bytes()).hexdigest(),
                  "run_id": "12", "attempt": "2", "proofs": proofs}
        cases = (
            (run, report, ["tree", "tree"], "GO"),
            (run, report, ["tree", "other"], "L’arbre"),
            (None, report, ["tree", "tree"], "absente"),
            ({**run, "status": "queued"}, report, ["tree", "tree"], "en cours"),
            ({**run, "conclusion": "cancelled"}, report, ["tree", "tree"], "sans succès"),
            ({**run, "conclusion": "failure"}, report, ["tree", "tree"], "sans succès"),
            (run, {**report, "sha": "stale"}, ["tree", "tree"], "Identité"),
            (run, {**report, "profile": "FABRIC"}, ["tree", "tree"], "Identité"),
            (run, {**report, "attempt": "1"}, ["tree", "tree"], "Identité"),
            (run, {**report, "decision": "NO-GO"}, ["tree", "tree"], "Identité"),
            (run, {**report, "proofs": {}}, ["tree", "tree"], "Couverture"),
            (run, {**report, "proofs": {**proofs, "current-archives": {"status": "FAIL"}}},
             ["tree", "tree"], "obligatoire"),
            (run, {**report, "proofs": {**proofs, "endurance.yml": {"status": "NON_QUALIFIANT"}}},
             ["tree", "tree"], "obligatoire"),
        )
        for run_case, report_case, trees, expected in cases:
            with self.subTest(expected=expected, report=report_case), patch.object(
                fabric_pr_gate.subprocess, "check_output", side_effect=trees,
            ), patch.object(fabric_pr_gate, "api", return_value=[run_case] if run_case else []), patch.object(
                fabric_pr_gate, "read_report", return_value=report_case,
            ), patch.object(fabric_pr_gate, "verify_archives"), patch.object(
                fabric_pr_gate, "verify_independent_campaigns",
            ):
                if expected == "GO":
                    self.assertEqual(fabric_pr_gate.verify("owner/repo", head, branch)["decision"], "GO")
                else:
                    with self.assertRaisesRegex(ValueError, expected):
                        fabric_pr_gate.verify("owner/repo", head, branch)

    def test_report_zip_must_match_github_digest(self):
        import fabric_pr_gate

        run = {"id": 12, "head_sha": "candidate", "run_started_at": "2026-11-26T18:00:00Z"}
        artifact = {"id": 42, "name": "proof", "digest": "sha256:" + "a" * 64, "expired": False,
                    "size_in_bytes": 20, "workflow_run": {"id": 12, "head_sha": "candidate"},
                    "created_at": "2026-11-26T18:00:00Z"}
        with patch.object(fabric_pr_gate, "api", return_value=[artifact]), patch.object(
            fabric_pr_gate.subprocess, "check_output", return_value=b"wrong archive",
        ), self.assertRaisesRegex(ValueError, "Empreinte"):
            fabric_pr_gate.read_report("api", run, "proof", "rapport.json")

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

    def test_ladybird_artifact_only_does_not_suppress_qualification(self):
        workflow = "ladybird-native-browser.yml"
        common = {
            "head_sha": "candidate", "head_branch": "topic", "event": "workflow_dispatch",
            "path": ".github/workflows/" + workflow, "name": "ladybird-native-browser",
        }
        production = {**common, "run_number": 8, "display_title": "ladybird-artefact recipe"}
        strict = {**common, "run_number": 7, "display_title": "ladybird-native-browser"}
        with patch.object(fabric_merge_gate, "api", return_value=[production]):
            self.assertIsNone(fabric_merge_gate.latest_run("owner/repo", "candidate", "topic", workflow))
        with patch.object(fabric_merge_gate, "api", return_value=[strict, production]):
            self.assertEqual(fabric_merge_gate.latest_run("owner/repo", "candidate", "topic", workflow), strict)
