"""deploy/cubebot's control CA: kept while good, reissued before it stops working."""

import subprocess
import tempfile
import unittest
from pathlib import Path

CUBEBOT = Path(__file__).resolve().parents[2] / "deploy/cubebot"


def openssl(*args, cwd):
    subprocess.run(["openssl", *args], check=True, capture_output=True, cwd=cwd)


class DevCaTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.directory = Path(temporary.name) / "ca"

    def issue(self, names="DNS:one.test"):
        subprocess.run([str(CUBEBOT), "dev-ca", str(self.directory), names], check=True)
        openssl("verify", "-CAfile", "ca.crt", "server.crt", cwd=self.directory)
        return self.read("ca.crt"), self.read("server.crt")

    def read(self, name):
        return (self.directory / name).read_bytes()

    def test_a_good_pair_is_kept(self):
        self.assertEqual(self.issue(), self.issue())

    def test_changed_names_reissue_the_certificate_under_the_same_ca(self):
        ca, server = self.issue()
        new_ca, new_server = self.issue("DNS:two.test")
        self.assertEqual(ca, new_ca)
        self.assertNotEqual(server, new_server)

    def test_a_certificate_near_expiry_is_reissued_under_the_same_ca(self):
        ca, server = self.issue()
        openssl(
            *("req", "-new", "-key", "server.key", "-subj", "/CN=old", "-out", "old"),
            cwd=self.directory,
        )
        openssl(
            *("x509", "-req", "-in", "old", "-CA", "ca.crt", "-CAkey", "ca.key"),
            *("-CAcreateserial", "-out", "server.crt", "-days", "1"),
            cwd=self.directory,
        )
        new_ca, new_server = self.issue()
        self.assertEqual(ca, new_ca)
        self.assertNotEqual(server, new_server)

    def test_a_ca_near_expiry_is_replaced_with_its_certificate(self):
        ca, server = self.issue()
        openssl(
            *("req", "-x509", "-key", "ca.key", "-subj", "/CN=old"),
            *("-out", "ca.crt", "-days", "1"),
            cwd=self.directory,
        )
        new_ca, new_server = self.issue()
        self.assertNotEqual(ca, new_ca)
        self.assertNotEqual(server, new_server)


if __name__ == "__main__":
    unittest.main()
