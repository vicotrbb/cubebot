//! Versioned authenticated encryption; installation keys never enter PostgreSQL.
use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, AeadCore, OsRng, Payload},
};

#[derive(Clone)]
pub struct Credentials(Aes256Gcm);

#[derive(Debug)]
pub struct CredentialError;

impl Credentials {
    /// Read exactly 32 raw bytes from operator-provisioned private key storage.
    pub fn from_file(path: &std::path::Path) -> Result<Self, CredentialError> {
        use std::io::Read;
        use std::os::unix::fs::MetadataExt;
        // Kubernetes projects Secret data through symlinks. Resolve once, then read the
        // private versioned file, never re-following a concurrently changed final symlink.
        let resolved = std::fs::canonicalize(path).map_err(|_| CredentialError)?;
        let mut file = std::fs::File::open(resolved).map_err(|_| CredentialError)?;
        let metadata = file.metadata().map_err(|_| CredentialError)?;
        if !metadata.is_file() || metadata.mode() & 0o077 != 0 || metadata.len() != 32 {
            return Err(CredentialError);
        }
        let mut key = [0; 32];
        file.read_exact(&mut key).map_err(|_| CredentialError)?;
        Ok(Self::new(key))
    }

    pub fn new(key: [u8; 32]) -> Self {
        Self(Aes256Gcm::new((&key).into()))
    }

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
            .map_err(|_| CredentialError)?;
        // Format 1 is AES-256-GCM, 96-bit nonce followed by ciphertext and tag.
        let mut record = vec![1];
        record.extend_from_slice(&nonce);
        record.extend_from_slice(&encrypted);
        Ok(record)
    }

    pub fn decrypt(&self, record: &[u8], context: &[u8]) -> Result<Vec<u8>, CredentialError> {
        if record.len() < 29 || record[0] != 1 {
            return Err(CredentialError);
        }
        self.0
            .decrypt(
                Nonce::from_slice(&record[1..13]),
                Payload {
                    msg: &record[13..],
                    aad: context,
                },
            )
            .map_err(|_| CredentialError)
    }
}
