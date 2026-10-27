"""Keep qualification artifacts out of source control."""

import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "tests/system"))
import cluster_admission as admission  # noqa: E402
from cluster_chat import evidence_directory  # noqa: E402
from cluster_chat_setup import certificate  # noqa: E402


class EvidenceLocationTests(unittest.TestCase):
    def test_rejects_checkout_and_its_descendants(self):
        checkout = Path(__file__).resolve().parents[2]
        for path in (checkout, checkout / "target" / "qualification"):
            with self.assertRaises(ValueError):
                evidence_directory(path)

    def test_accepts_external_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            self.assertEqual(evidence_directory(directory), Path(directory).resolve())

    def test_tls_uses_a_separate_ca_and_server_leaf(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = SimpleNamespace(
                artifacts=Path(directory), create=lambda *_a, **_k: None
            )
            certificate(fixture)
            private = Path(directory) / "private"
            self.assertTrue((private / "ca.crt").is_file())
            self.assertNotEqual(
                (private / "ca.crt").read_bytes(), (private / "tls.crt").read_bytes()
            )
            admission.run(
                "openssl",
                "verify",
                "-x509_strict",
                "-CAfile",
                str(private / "ca.crt"),
                str(private / "tls.crt"),
            )

    def test_source_fingerprint_tracks_frontend_and_browser_code(self):
        with tempfile.TemporaryDirectory() as directory:
            for name in ("fixture.tsx", "fixture.mjs", "bun.lock", ".dockerignore"):
                source = Path(directory) / name
                source.write_text("before")
                with patch.object(admission, "run", return_value=str(source)):
                    before = admission.fingerprint()
                    source.write_text("after")
                    self.assertNotEqual(before, admission.fingerprint())
