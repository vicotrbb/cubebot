"""Verify fresh-report creation and fail-visible cleanup at the Docker CLI boundary."""

import importlib.util
import io
import json
import tempfile
import unittest
from contextlib import redirect_stderr
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

SOURCE = Path(__file__).resolve().parents[2] / "tests/system/container_isolation.py"
SPEC = importlib.util.spec_from_file_location("container_isolation", SOURCE)
PROBE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PROBE)


class ContainerIsolationTests(unittest.TestCase):
    def test_report_creates_missing_parent_and_preserves_observations(self):
        with tempfile.TemporaryDirectory() as root:
            target = Path(root) / "missing" / "report.json"
            PROBE.write_report(target, {"observed": True})
            self.assertEqual(json.loads(target.read_text()), {"observed": True})

    def test_cleanup_attempts_all_owned_resources_and_returns_failures(self):
        denied = SimpleNamespace(returncode=1, stderr="permission denied", stdout="")
        with patch.object(PROBE.subprocess, "run", return_value=denied) as command:
            failures = PROBE.cleanup("owned-fixture", "owned-builder")
        self.assertEqual(command.call_count, 6)
        self.assertEqual(len(failures), 6)
        self.assertTrue(all("owned-" in failure for failure in failures))

    def test_missing_resources_are_already_clean(self):
        missing = [
            "No such container: owned",
            "no such volume",
            'no builder "owned-build" found',
        ]
        missing.extend(["No such image: owned"] * 3)
        results = [
            SimpleNamespace(returncode=1, stderr=text, stdout="") for text in missing
        ]
        with patch.object(PROBE.subprocess, "run", side_effect=results):
            self.assertEqual(PROBE.cleanup("owned", "owned-build"), [])

    def test_cleanup_failure_is_reported_without_masking_primary_failure(self):
        failure = SimpleNamespace(
            returncode=1, stderr="primary dependency failure", stdout=""
        )
        errors = io.StringIO()
        with (
            patch.object(PROBE.subprocess, "run", return_value=failure),
            redirect_stderr(errors),
        ):
            with self.assertRaisesRegex(RuntimeError, "primary dependency failure"):
                PROBE.main()
        self.assertIn("cleanup", errors.getvalue().lower())
        self.assertIn("cubebot-isolation-", errors.getvalue())
