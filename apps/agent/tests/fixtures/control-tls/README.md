# Test-only TLS certificates

These public test fixtures establish localhost TLS trust and hostname verification in `control_tls.rs`. `server.key` is intentionally public test material and must never be used by an installation. The signing CA private key was discarded after generation. Certificates expire in October 2036; replace this fixture before then.
