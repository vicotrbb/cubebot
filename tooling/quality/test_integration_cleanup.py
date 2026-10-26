"""Preserve integration failures while attempting all disposable database cleanup."""

import importlib.util
import io
import sys
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


if __name__ == "__main__":
    unittest.main()
