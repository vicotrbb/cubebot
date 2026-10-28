"""Verify fail-visible cleanup of owned images at the Docker CLI boundary."""

import importlib.util
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

SOURCE = Path(__file__).resolve().parents[2] / "tests/system/images.py"
SPEC = importlib.util.spec_from_file_location("images", SOURCE)
IMAGES = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(IMAGES)


class ImageCleanupTests(unittest.TestCase):
    def test_cleanup_attempts_all_owned_resources_and_returns_failures(self):
        denied = SimpleNamespace(returncode=1, stderr="permission denied", stdout="")
        with patch.object(IMAGES.subprocess, "run", return_value=denied) as command:
            failures = IMAGES.cleanup("owned-fixture", "owned-builder")
        self.assertEqual(command.call_count, 4)
        self.assertEqual(len(failures), 4)
        self.assertTrue(all("owned-" in failure for failure in failures))

    def test_missing_resources_are_already_clean(self):
        missing = [
            "No such container: owned",
            "no such volume",
            'no builder "owned-build" found',
            "No such image: owned",
        ]
        results = [
            SimpleNamespace(returncode=1, stderr=text, stdout="") for text in missing
        ]
        with patch.object(IMAGES.subprocess, "run", side_effect=results):
            self.assertEqual(IMAGES.cleanup("owned", "owned-build"), [])


if __name__ == "__main__":
    unittest.main()
