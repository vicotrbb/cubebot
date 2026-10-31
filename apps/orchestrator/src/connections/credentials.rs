//! Versioned authenticated encryption; installation keys never enter PostgreSQL.
use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, AeadCore, OsRng, Payload},
};

#[derive(Clone)]
pub struct Credentials(Aes256Gcm);

/// Carries the key file's I/O error; AEAD failures are deliberately opaque and carry none.
#[derive(Debug)]
pub struct CredentialError(Option<std::io::Error>);

impl std::fmt::Display for CredentialError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("credential unavailable")
    }
}

impl std::error::Error for CredentialError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.as_ref().map(|error| error as _)
    }
}

impl From<std::io::Error> for CredentialError {
    fn from(error: std::io::Error) -> Self {
        Self(Some(error))
    }
}

impl Credentials {
    /// Read exactly 32 raw bytes from operator-provisioned private key storage.
    #[tracing::instrument(skip_all)]
    pub fn from_file(path: &std::path::Path) -> Result<Self, CredentialError> {
        use std::io::Read;
        use std::os::unix::fs::MetadataExt;
        // Kubernetes projects Secret data through symlinks. Resolve once, then read the
        // private versioned file, never re-following a concurrently changed final symlink.
        let resolved = std::fs::canonicalize(path)?;
        let mut file = std::fs::File::open(resolved)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.mode() & 0o077 != 0 || metadata.len() != 32 {
            return Err(CredentialError(None));
        }
        let mut key = [0; 32];
        file.read_exact(&mut key)?;
        Ok(Self::new(key))
    }

    pub fn new(key: [u8; 32]) -> Self {
        Self(Aes256Gcm::new((&key).into()))
    }

    #[tracing::instrument(skip_all)]
    pub fn encrypt(&self, secret: &[u8], context: &[u8]) -> Result<Vec<u8>, CredentialError> {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let encrypted = self
            .0
            .encrypt(
                &nonce,
                Payload {
                    msg: secret,
                    aad: context,
                },
            )
            .map_err(|_| CredentialError(None))?;
        // Format 1 is AES-256-GCM, 96-bit nonce followed by ciphertext and tag.
        let mut record = vec![1];
        record.extend_from_slice(&nonce);
        record.extend_from_slice(&encrypted);
        Ok(record)
    }

    #[tracing::instrument(skip_all)]
    pub fn decrypt(&self, record: &[u8], context: &[u8]) -> Result<Vec<u8>, CredentialError> {
        if record.len() < 29 || record[0] != 1 {
            return Err(CredentialError(None));
        }
        self.0
            .decrypt(
                Nonce::from_slice(&record[1..13]),
                Payload {
                    msg: &record[13..],
                    aad: context,
                },
            )
            .map_err(|_| CredentialError(None))
    }
}
