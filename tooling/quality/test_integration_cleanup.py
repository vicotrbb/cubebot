"""Preserve integration failures while attempting all disposable database cleanup."""

import importlib.util
import io
import sys
import tempfile
import unittest
from contextlib import redirect_stderr
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

SOURCE = Path(__file__).resolve().parents[2] / "tests/system/integration.py"
SPEC = importlib.util.spec_from_file_location("database_integration", SOURCE)
RUNNER = importlib.util.module_from_spec(SPEC)
with patch.object(sys, "path", [str(SOURCE.parent), *sys.path]):
    SPEC.loader.exec_module(RUNNER)


class DatabaseCleanupTests(unittest.TestCase):
    def setUp(self):
        permissions = patch.object(RUNNER.os, "chmod")
        permissions.start()
        self.addCleanup(permissions.stop)

    def test_lost_start_acknowledgement_still_attempts_owned_cleanup(self):
        primary = RuntimeError("start response lost")
        with (
            patch.object(RUNNER, "run", side_effect=[primary, None]),
            patch.object(RUNNER, "restore_data_owner") as owner,
            redirect_stderr(io.StringIO()),
        ):
            with self.assertRaises(RuntimeError) as outcome:
                RUNNER.run_database("owned", {}, "fixture", "/fixture", lambda *_: None)
        self.assertIs(outcome.exception, primary)
        owner.assert_called_once()

    def test_workflow_error_survives_both_cleanup_failures(self):
        primary = RuntimeError("primary admission failure")

        def workflow(*_):
            raise primary

        results = [
            None,
            SimpleNamespace(stdout="127.0.0.1:54321"),
            RuntimeError("stop failed"),
        ]
        errors = io.StringIO()
        with (
            patch.object(RUNNER, "run", side_effect=results),
            patch.object(RUNNER, "wait_ready"),
            patch.object(
                RUNNER, "restore_data_owner", side_effect=RuntimeError("owner failed")
            ) as owner,
            redirect_stderr(errors),
        ):
            with self.assertRaises(RuntimeError) as outcome:
                RUNNER.run_database("owned", {}, "fixture", "/fixture", workflow)
        self.assertIs(outcome.exception, primary)
        owner.assert_called_once()
        self.assertIn("stop failed", errors.getvalue())
        self.assertIn("owner failed", errors.getvalue())

    def test_successful_workflow_still_fails_on_cleanup_error(self):
        results = [
            None,
            SimpleNamespace(stdout="127.0.0.1:54321"),
            RuntimeError("stop failed"),
        ]
        with (
            patch.object(RUNNER, "run", side_effect=results),
            patch.object(RUNNER, "wait_ready"),
            patch.object(RUNNER, "restore_data_owner") as owner,
            redirect_stderr(io.StringIO()),
        ):
            with self.assertRaises(RuntimeError):
                RUNNER.run_database("owned", {}, "fixture", "/fixture", lambda *_: None)
        owner.assert_called_once()


class DatabaseMountPermissionTests(unittest.TestCase):
    def test_parent_allows_container_uid_traversal_before_launch(self):
        observed = []
        with tempfile.TemporaryDirectory() as data:

            def launch(*_args, **_kwargs):
                observed.append(Path(data).stat().st_mode & 0o777)
                raise RuntimeError("controlled startup stop")

            with (
                patch.object(RUNNER, "run", side_effect=launch),
                patch.object(RUNNER, "cleanup_database", return_value=[]),
            ):
                with self.assertRaisesRegex(RuntimeError, "controlled startup stop"):
                    RUNNER.run_database("owned", {}, "fixture", data, lambda *_: None)
        self.assertEqual(observed, [0o711])


if __name__ == "__main__":
    unittest.main()
