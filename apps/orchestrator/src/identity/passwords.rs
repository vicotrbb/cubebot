use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use rand_core::{OsRng, RngCore};

use super::IdentityError;

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
    let password = password.to_owned();
    tokio::task::spawn_blocking(move || {
        Argon2::default()
            .hash_password(password.as_bytes(), &SaltString::generate(&mut OsRng))
            .map(|hash| hash.to_string())
            .map_err(|_| IdentityError::Password)
    })
    .await
    .map_err(|_| IdentityError::Password)?
}

pub async fn verify(password: &str, verifier: String) -> Result<(), IdentityError> {
    let password = password.to_owned();
    tokio::task::spawn_blocking(move || {
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
