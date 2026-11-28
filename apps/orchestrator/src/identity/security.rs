//! What a person manages about their own sign-in: password, sessions, personal API tokens and
//! a time-based second factor (RFC 6238), plus the administrator's recovery tools for them.

use ring::hmac;
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::{Actor, Identity, IdentityError, Receipt, commands::Command, members, passwords};
use crate::identity::sessions::digest;

const SEAL: &[u8] = b"cubebot.identity.totp";
const STEP_SECONDS: i64 = 30;
const RECOVERY_CODES: usize = 10;

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SessionSummary {
    pub id: Uuid,
    /// Set for a personal API token; browser sign-ins have none.
    pub name: Option<String>,
    pub current: bool,
    pub created_at_unix: i64,
    pub expires_at_unix: i64,
    pub last_seen_at_unix: Option<i64>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct Enrollment {
    /// Base32, for typing into an authenticator app.
    pub secret: String,
    /// An `otpauth://` URI for a QR code.
    pub uri: String,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NewToken {
    pub name: String,
    /// 1 to 365.
    pub days: i32,
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AccountRequest {
    pub request_id: Uuid,
    pub id: Uuid,
}

fn base32(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = String::new();
    let (mut buffer, mut bits) = (0u32, 0u32);
    for &byte in bytes {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buffer >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buffer << (5 - bits)) & 31) as usize] as char);
    }
    out
}

fn hotp(secret: &[u8], counter: u64) -> u32 {
    let key = hmac::Key::new(hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, secret);
    let tag = hmac::sign(&key, &counter.to_be_bytes());
    let tag = tag.as_ref();
    let offset = usize::from(tag[19] & 0x0f);
    let value = u32::from_be_bytes([
        tag[offset],
        tag[offset + 1],
        tag[offset + 2],
        tag[offset + 3],
    ]);
    (value & 0x7fff_ffff) % 1_000_000
}

/// The step a six-digit code matches, one step either side of now, never at or before `last`.
fn matching_step(secret: &[u8], code: &str, now: i64, last: Option<i64>) -> Option<i64> {
    if code.len() != 6 || !code.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let wanted: u32 = code.parse().ok()?;
    let current = now.div_euclid(STEP_SECONDS);
    (current - 1..=current + 1)
        .filter(|step| last.is_none_or(|last| *step > last) && *step >= 0)
        .find(|step| hotp(secret, u64::try_from(*step).unwrap_or_default()) == wanted)
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        })
}

fn recovery_code() -> String {
    let raw = passwords::token();
    format!("{}-{}", &raw[..5], &raw[5..10])
}

fn normalized(code: &str) -> String {
    code.trim().to_ascii_lowercase()
}

impl Identity {
    fn seal(&self, secret: &[u8]) -> Result<Vec<u8>, IdentityError> {
        let credentials = self
            .credentials
            .as_ref()
            .ok_or(IdentityError::CredentialUnavailable(None))?;
        credentials
            .encrypt(secret, SEAL)
            .map_err(|error| IdentityError::CredentialUnavailable(Some(error.into())))
    }

    fn unseal(&self, sealed: &[u8]) -> Result<Vec<u8>, IdentityError> {
        let credentials = self
            .credentials
            .as_ref()
            .ok_or(IdentityError::CredentialUnavailable(None))?;
        credentials
            .decrypt(sealed, SEAL)
            .map_err(|error| IdentityError::CredentialUnavailable(Some(error.into())))
    }

    /// Checks the second factor inside the sign-in transaction, whose row lock makes a code
    /// usable once. Accounts without two-factor pass.
    pub(super) async fn second_factor(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        account: Uuid,
        code: Option<&str>,
    ) -> Result<(), IdentityError> {
        let row = sqlx::query!(
            r#"
            SELECT totp_secret AS "secret!", totp_last_step
            FROM human_accounts
            WHERE id = $1 AND totp_enabled_at IS NOT NULL
            "#,
            account,
        )
        .fetch_optional(&mut **transaction)
        .await?;
        let Some(row) = row else {
            return Ok(());
        };
        let code = code.ok_or(IdentityError::TwoFactorRequired)?;
        let secret = self.unseal(&row.secret)?;
        let now = unix_now();
        if let Some(step) = matching_step(&secret, code.trim(), now, row.totp_last_step) {
            sqlx::query!(
                "UPDATE human_accounts SET totp_last_step = $2 WHERE id = $1",
                account,
                step,
            )
            .execute(&mut **transaction)
            .await?;
            return Ok(());
        }
        let used = sqlx::query_scalar!(
            "UPDATE human_recovery_codes SET used_at = clock_timestamp() WHERE account_id = $1 AND code_hash = $2 AND used_at IS NULL RETURNING account_id",
            account,
            digest(&normalized(code)),
        )
        .fetch_optional(&mut **transaction)
        .await?;
        used.map(|_| ()).ok_or(IdentityError::Unauthorized)
    }

    /// Starts two-factor enrollment; it takes effect once `confirm_two_factor` sees a code.
    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn enroll_two_factor(&self, actor: &Actor) -> Result<Enrollment, IdentityError> {
        let mut secret = [0u8; 20];
        rand_core::RngCore::fill_bytes(&mut rand_core::OsRng, &mut secret);
        let sealed = self.seal(&secret)?;
        let login = sqlx::query_scalar!(
            "UPDATE human_accounts SET totp_secret = $2 WHERE id = $1 AND active AND totp_enabled_at IS NULL RETURNING login",
            actor.id,
            sealed,
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or(IdentityError::Conflict)?;
        let secret = base32(&secret);
        Ok(Enrollment {
            uri: format!(
                "otpauth://totp/Cubebot:{login}?secret={secret}&issuer=Cubebot&digits=6&period=30"
            ),
            secret,
        })
    }

    /// Turns two-factor on and returns single-use recovery codes, shown this once.
    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn confirm_two_factor(
        &self,
        actor: &Actor,
        code: &str,
    ) -> Result<Vec<String>, IdentityError> {
        // Codes are guessable; each try spends from the same budget as a password.
        self.reserve_password_check(actor.id).await?;
        let mut transaction = self.pool.begin().await?;
        let sealed = sqlx::query_scalar!(
            r#"SELECT totp_secret AS "secret!" FROM human_accounts WHERE id = $1 AND active AND totp_enabled_at IS NULL AND totp_secret IS NOT NULL FOR UPDATE"#,
            actor.id,
        )
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(IdentityError::Conflict)?;
        let secret = self.unseal(&sealed)?;
        let now = unix_now();
        let step =
            matching_step(&secret, code.trim(), now, None).ok_or(IdentityError::InvalidInput)?;
        sqlx::query!(
            "UPDATE human_accounts SET totp_enabled_at = clock_timestamp(), totp_last_step = $2 WHERE id = $1",
            actor.id,
            step,
        )
        .execute(&mut *transaction)
        .await?;
        let codes = replace_recovery_codes(&mut transaction, actor.id).await?;
        let event = super::audit::Event::by(actor.id, "account.two_factor_on").on(actor.id);
        super::audit::record(&mut *transaction, &event).await?;
        transaction.commit().await?;
        Ok(codes)
    }

    /// Turns two-factor off; a current code or a recovery code proves it is the owner.
    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn disable_two_factor(&self, actor: &Actor, code: &str) -> Result<(), IdentityError> {
        self.reserve_password_check(actor.id).await?;
        let mut transaction = self.pool.begin().await?;
        sqlx::query_scalar!(
            "SELECT id FROM human_accounts WHERE id = $1 AND active FOR UPDATE",
            actor.id
        )
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(IdentityError::Unauthorized)?;
        match self
            .second_factor(&mut transaction, actor.id, Some(code))
            .await
        {
            Err(IdentityError::Unauthorized) => return Err(IdentityError::InvalidInput),
            other => other?,
        }
        clear_two_factor(&mut transaction, actor.id).await?;
        let event = super::audit::Event::by(actor.id, "account.two_factor_off").on(actor.id);
        super::audit::record(&mut *transaction, &event).await?;
        transaction.commit().await?;
        Ok(())
    }

    /// An administrator removes someone's lost second factor.
    #[tracing::instrument(skip_all, fields(actor = %actor.id, request = %input.request_id))]
    pub async fn reset_two_factor(
        &self,
        actor: &Actor,
        input: &AccountRequest,
    ) -> Result<Receipt, IdentityError> {
        let command = Command::new(actor, "account.two_factor_reset", input.request_id, input)?;
        let (mut transaction, previous) = command.begin(&self.pool).await?;
        if let Some(receipt) = previous {
            return Ok(receipt);
        }
        let subject = members::subject(&mut transaction, input.id).await?;
        members::guard_subject(&mut transaction, actor, &subject, false).await?;
        clear_two_factor(&mut transaction, input.id).await?;
        command
            .commit(
                transaction,
                Receipt {
                    id: input.id,
                    revision: 1,
                },
            )
            .await
    }

    /// Changes the actor's own password and ends every other session and token.
    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn change_password(
        &self,
        actor: &Actor,
        current: &str,
        new: &str,
        keep_token: &str,
    ) -> Result<(), IdentityError> {
        let account = sqlx::query!(
            r#"SELECT login, password_hash AS "hash!" FROM human_accounts WHERE id = $1 AND active AND password_hash IS NOT NULL"#,
            actor.id,
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or(IdentityError::Conflict)?;
        passwords::validate(&account.login, new)?;
        if current.len() > 1024 {
            return Err(IdentityError::InvalidInput);
        }
        self.reserve_password_check(actor.id).await?;
        match passwords::verify(current, account.hash).await {
            Err(IdentityError::Unauthorized) => return Err(IdentityError::InvalidInput),
            other => other?,
        }
        let verifier = passwords::hash(new).await?;
        let mut transaction = self.pool.begin().await?;
        set_password(
            &mut transaction,
            actor.id,
            &verifier,
            Some(&digest(keep_token)),
        )
        .await?;
        let event = super::audit::Event::by(actor.id, "account.password_change").on(actor.id);
        super::audit::record(&mut *transaction, &event).await?;
        transaction.commit().await?;
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn sessions(
        &self,
        actor: &Actor,
        current_token: &str,
    ) -> Result<Vec<SessionSummary>, IdentityError> {
        Ok(sqlx::query_as!(
            SessionSummary,
            r#"
            SELECT id, name, token_hash = $2 AS "current!",
                   EXTRACT(EPOCH FROM created_at)::bigint AS "created_at_unix!",
                   EXTRACT(EPOCH FROM expires_at)::bigint AS "expires_at_unix!",
                   EXTRACT(EPOCH FROM last_seen_at)::bigint AS last_seen_at_unix
            FROM human_sessions
            WHERE account_id = $1 AND expires_at > clock_timestamp()
            ORDER BY created_at DESC
            "#,
            actor.id,
            digest(current_token),
        )
        .fetch_all(&self.pool)
        .await?)
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, session = %id))]
    pub async fn end_session(&self, actor: &Actor, id: Uuid) -> Result<(), IdentityError> {
        let ended = sqlx::query_scalar!(
            "DELETE FROM human_sessions WHERE id = $1 AND account_id = $2 RETURNING id",
            id,
            actor.id,
        )
        .fetch_optional(&self.pool)
        .await?;
        ended.ok_or(IdentityError::NotFound)?;
        self.audit(super::audit::Event::by(actor.id, "session.end").on(id))
            .await;
        Ok(())
    }

    /// A named, long-lived bearer token with the person's own authority; shown once. It ends
    /// with the single sign-on policy the calling session came from, as that session does.
    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn create_token(
        &self,
        actor: &Actor,
        input: &NewToken,
        caller: &str,
    ) -> Result<String, IdentityError> {
        let name = input.name.trim();
        if name.is_empty()
            || name.chars().count() > 100
            || name.chars().any(char::is_control)
            || !(1..=365).contains(&input.days)
        {
            return Err(IdentityError::InvalidInput);
        }
        if actor.role == super::Role::Guest {
            return Err(IdentityError::Forbidden);
        }
        let token = passwords::token();
        let id = sqlx::query_scalar!(
            r#"
            INSERT INTO human_sessions (token_hash, account_id, expires_at, name, oidc_policy_revision)
            SELECT $1, a.id, clock_timestamp() + make_interval(days => $3), $4, s.oidc_policy_revision
            FROM human_accounts a JOIN human_sessions s ON s.account_id = a.id AND s.token_hash = $5
            WHERE a.id = $2 AND a.active
            RETURNING id
            "#,
            digest(&token),
            actor.id,
            input.days,
            name,
            digest(caller),
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or(IdentityError::Unauthorized)?;
        self.audit(
            super::audit::Event::by(actor.id, "session.token_create")
                .on(id)
                .detail(serde_json::json!({"name": name, "days": input.days})),
        )
        .await;
        Ok(token)
    }

    /// A one-day link with which someone sets a new password; it ends their sessions.
    #[tracing::instrument(skip_all, fields(actor = %actor.id, request = %input.request_id))]
    pub async fn create_password_reset(
        &self,
        actor: &Actor,
        input: &AccountRequest,
    ) -> Result<String, IdentityError> {
        let command = Command::new(actor, "account.password_reset", input.request_id, input)?;
        let (mut transaction, previous) = command.begin(&self.pool).await?;
        if previous.is_some() {
            // The token is not stored in the clear, so a retry cannot return it.
            return Err(IdentityError::Conflict);
        }
        let subject = members::subject(&mut transaction, input.id).await?;
        members::guard_subject(&mut transaction, actor, &subject, false).await?;
        let token = passwords::token();
        let id = sqlx::query_scalar!(
            "INSERT INTO password_resets (account_id, token_hash, created_by) VALUES ($1, $2, $3) RETURNING id",
            input.id,
            digest(&token),
            actor.id,
        )
        .fetch_one(&mut *transaction)
        .await?;
        command
            .commit(transaction, Receipt { id, revision: 1 })
            .await?;
        Ok(token)
    }

    #[tracing::instrument(skip_all)]
    pub async fn redeem_password_reset(
        &self,
        token: &str,
        password: &str,
    ) -> Result<(), IdentityError> {
        if token.len() != 64 {
            return Err(IdentityError::Unauthorized);
        }
        let login = sqlx::query_scalar!(
            r#"
            SELECT a.login FROM password_resets r JOIN human_accounts a ON a.id = r.account_id AND a.active
            WHERE r.token_hash = $1 AND r.used_at IS NULL AND r.expires_at > clock_timestamp()
              AND reset_still_authorized(r.account_id, r.created_by)
            "#,
            digest(token),
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or(IdentityError::Unauthorized)?;
        passwords::validate(&login, password)?;
        let verifier = passwords::hash(password).await?;
        let mut transaction = self.pool.begin().await?;
        let account = sqlx::query_scalar!(
            "UPDATE password_resets SET used_at = clock_timestamp() WHERE token_hash = $1 AND used_at IS NULL AND expires_at > clock_timestamp() AND reset_still_authorized(account_id, created_by) RETURNING account_id",
            digest(token),
        )
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(IdentityError::Unauthorized)?;
        set_password(&mut transaction, account, &verifier, None).await?;
        let event = super::audit::Event::by(account, "account.password_reset_used").on(account);
        super::audit::record(&mut *transaction, &event).await?;
        transaction.commit().await?;
        Ok(())
    }
}

async fn set_password(
    transaction: &mut Transaction<'_, Postgres>,
    account: Uuid,
    verifier: &str,
    keep: Option<&[u8]>,
) -> Result<(), IdentityError> {
    sqlx::query!(
        "UPDATE human_accounts SET password_hash = $2, auth_attempts = 0, revision = revision + 1 WHERE id = $1",
        account,
        verifier,
    )
    .execute(&mut **transaction)
    .await?;
    sqlx::query!(
        "DELETE FROM human_sessions WHERE account_id = $1 AND token_hash IS DISTINCT FROM $2",
        account,
        keep,
    )
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn clear_two_factor(
    transaction: &mut Transaction<'_, Postgres>,
    account: Uuid,
) -> Result<(), IdentityError> {
    sqlx::query!(
        "UPDATE human_accounts SET totp_secret = NULL, totp_enabled_at = NULL, totp_last_step = NULL WHERE id = $1",
        account,
    )
    .execute(&mut **transaction)
    .await?;
    sqlx::query!(
        "DELETE FROM human_recovery_codes WHERE account_id = $1",
        account
    )
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn replace_recovery_codes(
    transaction: &mut Transaction<'_, Postgres>,
    account: Uuid,
) -> Result<Vec<String>, IdentityError> {
    sqlx::query!(
        "DELETE FROM human_recovery_codes WHERE account_id = $1",
        account
    )
    .execute(&mut **transaction)
    .await?;
    let codes: Vec<String> = (0..RECOVERY_CODES).map(|_| recovery_code()).collect();
    for code in &codes {
        sqlx::query!(
            "INSERT INTO human_recovery_codes (account_id, code_hash) VALUES ($1, $2)",
            account,
            digest(code),
        )
        .execute(&mut **transaction)
        .await?;
    }
    Ok(codes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_rfc_6238_sha1_vectors() {
        // RFC 6238 appendix B, truncated to six digits.
        let secret = b"12345678901234567890";
        assert_eq!(hotp(secret, 59 / 30), 287_082);
        assert_eq!(hotp(secret, 1_111_111_109 / 30), 81_804);
        assert_eq!(hotp(secret, 2_000_000_000 / 30), 279_037);
    }

    #[test]
    fn accepts_neighbouring_steps_once() {
        let secret = b"12345678901234567890";
        let now = 1_111_111_109;
        let step = now / 30;
        let code = format!("{:06}", hotp(secret, (step - 1) as u64));
        assert_eq!(matching_step(secret, &code, now, None), Some(step - 1));
        assert_eq!(matching_step(secret, &code, now, Some(step - 1)), None);
        let late = format!("{:06}", hotp(secret, (step - 2) as u64));
        assert_eq!(matching_step(secret, &late, now, None), None);
        assert_eq!(matching_step(secret, "12a456", now, None), None);
    }

    #[test]
    fn encodes_base32_like_rfc_4648() {
        assert_eq!(base32(b"foobar"), "MZXW6YTBOI");
        assert_eq!(base32(b"f"), "MY");
    }
}
