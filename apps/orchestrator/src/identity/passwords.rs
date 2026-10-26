use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use rand_core::{OsRng, RngCore};

use super::IdentityError;

// Shared by hashing and verification. The permit stays with uncancellable CPU work.
static HASH_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);

pub fn validate_login(login: &str) -> Result<(), IdentityError> {
    let valid_login = login
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte));
    let valid_first = login
        .bytes()
        .next()
        .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit());
    if !(3..=64).contains(&login.len()) || !valid_login || !valid_first {
        return Err(IdentityError::InvalidInput);
    }
    Ok(())
}

pub fn validate(login: &str, password: &str) -> Result<(), IdentityError> {
    validate_login(login)?;
    if !(12..=1024).contains(&password.len()) {
        return Err(IdentityError::InvalidInput);
    }
    Ok(())
}

pub async fn hash(password: &str) -> Result<String, IdentityError> {
    let permit = HASH_SLOTS.try_acquire().map_err(|_| IdentityError::Busy)?;
    let password = password.to_owned();
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        Argon2::default()
            .hash_password(password.as_bytes(), &SaltString::generate(&mut OsRng))
            .map(|hash| hash.to_string())
            .map_err(|_| IdentityError::Password)
    })
    .await
    .map_err(|_| IdentityError::Password)?
}

pub async fn verify(password: &str, verifier: String) -> Result<(), IdentityError> {
    let permit = HASH_SLOTS.try_acquire().map_err(|_| IdentityError::Busy)?;
    let password = password.to_owned();
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let parsed = PasswordHash::new(&verifier).map_err(|_| IdentityError::Password)?;
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .map_err(|_| IdentityError::Unauthorized)
    })
    .await
    .map_err(|_| IdentityError::Password)?
}

pub fn token() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn saturated_hash_capacity_rejects_hash_and_verification_without_queueing() {
        let verifier = hash("a long real password").await.unwrap();
        let capacity = HASH_SLOTS.acquire_many(4).await.unwrap();
        assert!(matches!(
            hash("another real password").await,
            Err(IdentityError::Busy)
        ));
        assert!(matches!(
            verify("a long real password", verifier.clone()).await,
            Err(IdentityError::Busy)
        ));
        drop(capacity);
        verify("a long real password", verifier).await.unwrap();
    }
}
