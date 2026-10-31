#!/usr/bin/env python3
"""Regression tests for fail-closed C22 native artifact smoke eligibility."""
import copy
import importlib.util
from pathlib import Path
import unittest

MODULE = Path(__file__).resolve().parents[1] / "c22_native_preflight.py"
spec = importlib.util.spec_from_file_location("c22_native_preflight", MODULE)
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)

REPO = "bcharthur/bouchaud-os"
RUN_ID = 41140572088
RUN = {
    "id": RUN_ID, "name": "ladybird-native-browser",
    "status": "completed", "conclusion": "failure",
    "repository": {"full_name": REPO},
    "head_repository": {"full_name": REPO},
    "head_branch": "chatgpt/active", "event": "push",
    "head_sha": "65549daceaad3ac84e23a16a4b7cf6a15f15286e",
}
JOBS = {"jobs": [
    {"name": gate.BUILD, "status": "completed", "conclusion": "success"},
    {"name": gate.REUSED, "status": "completed", "conclusion": "skipped"},
    {"name": gate.AVAILABLE, "status": "completed", "conclusion": "success"},
    {"name": gate.MEMORY, "status": "completed", "conclusion": "failure"},
    {"name": gate.VERDICT, "status": "completed", "conclusion": "failure"},
    {"name": "primitives", "status": "completed", "conclusion": "skipped"},
    {"name": "ladybird / browser-host smoke", "status": "completed", "conclusion": "success"},
]}
ARTIFACTS = {"artifacts": [{
    "id": 13022539892, "name": gate.ARTIFACT, "expired": False,
    "size_in_bytes": 433794313,
    "digest": "sha256:" + "e3" * 32,
    "workflow_run": {"id": RUN_ID},
}]}


class NativePreflightTests(unittest.TestCase):
    def setUp(self):
        self.run = copy.deepcopy(RUN)
        self.jobs = copy.deepcopy(JOBS)
        self.artifacts = copy.deepcopy(ARTIFACTS)

    def check(self):
        return gate.verify(self.run, self.jobs, self.artifacts, RUN_ID, REPO)

    def test_red_only_for_memory_and_verdict_can_supply_smoke(self):
        self.assertTrue(self.check())

    def test_all_green_producer_can_supply_smoke(self):
        self.run["conclusion"] = "success"
        for job in self.jobs["jobs"]:
            if job["conclusion"] == "failure":
                job["conclusion"] = "success"
        self.assertTrue(self.check())

    def test_other_failed_job_is_never_accepted(self):
        self.jobs["jobs"][-1]["conclusion"] = "failure"
        with self.assertRaises(ValueError):
            self.check()

    def test_memory_red_without_matching_verdict_is_rejected(self):
        self.jobs["jobs"][4]["conclusion"] = "success"
        with self.assertRaises(ValueError):
            self.check()

    def test_incomplete_build_is_rejected(self):
        self.jobs["jobs"][0]["status"] = "in_progress"
        with self.assertRaises(ValueError):
            self.check()

    def test_missing_successful_build_is_rejected(self):
        self.jobs["jobs"][0]["conclusion"] = "skipped"
        with self.assertRaises(ValueError):
            self.check()

    def test_wrong_repo_is_rejected(self):
        self.run["head_repository"]["full_name"] = "other/repo"
        with self.assertRaises(ValueError):
            self.check()

    def test_wrong_branch_is_rejected(self):
        self.run["head_branch"] = "main"
        with self.assertRaises(ValueError):
            self.check()

    def test_missing_artifact_is_rejected(self):
        self.artifacts["artifacts"] = []
        with self.assertRaises(ValueError):
            self.check()

    def test_expired_artifact_is_rejected(self):
        self.artifacts["artifacts"][0]["expired"] = True
        with self.assertRaises(ValueError):
            self.check()

    def test_wrong_artifact_run_is_rejected(self):
        self.artifacts["artifacts"][0]["workflow_run"]["id"] = RUN_ID - 1
        with self.assertRaises(ValueError):
            self.check()

    def test_artifact_without_sha256_is_rejected(self):
        self.artifacts["artifacts"][0]["digest"] = None
        with self.assertRaises(ValueError):
            self.check()

    def test_run_must_match_input(self):
        self.run["id"] = RUN_ID - 1
        with self.assertRaises(ValueError):
            self.check()

    def test_skipped_non_optional_job_is_rejected(self):
        self.jobs["jobs"][-1]["conclusion"] = "skipped"
        with self.assertRaises(ValueError):
            self.check()


if __name__ == "__main__":
    unittest.main()
