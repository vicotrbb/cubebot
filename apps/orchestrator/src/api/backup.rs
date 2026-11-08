//! Platform › Backup: settings, history, starting a backup and the one credential-key download.
use crate::{
    backup::{
        BackupSummary, Backups, WorkspaceRestore,
        restore::RestoreProgress,
        settings::{self, Fields, SaveBackup, Saved},
    },
    identity::{Identity, IdentityError, audit::Event},
};
use axum::{
    Extension, Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use serde::Serialize;
use uuid::Uuid;

#[derive(Clone)]
struct Api {
    identity: Identity,
    pool: sqlx::PgPool,
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(backup, save_backup, back_up_now, download_key, restore_progress))]
pub(super) struct Doc;

pub(super) fn router(pool: sqlx::PgPool, identity: Identity) -> Router {
    Router::new()
        .route("/api/installation/backup", get(backup).put(save_backup))
        .route("/api/installation/backup/run", post(back_up_now))
        .route("/api/installation/backup/key", post(download_key))
        .route("/api/restore", get(restore_progress))
        .with_state(Api { identity, pool })
}

#[derive(Serialize, utoipa::ToSchema)]
struct Defaults {
    region: &'static str,
    schedule: &'static str,
    keep_daily: u32,
    keep_weekly: u32,
    bandwidth_bytes: u64,
    concurrency: usize,
}

#[derive(Serialize, utoipa::ToSchema)]
struct KeyView {
    /// Whether this installation has a credential key at all.
    present: bool,
    downloaded_at_unix: Option<i64>,
    downloaded_by: Option<String>,
}

#[derive(Serialize, utoipa::ToSchema)]
struct BackupView {
    /// Saved here; a value in `deployment` wins over it.
    settings: Fields,
    secret_access_key_saved: bool,
    revision: i64,
    /// Set by the deployment and read-only here.
    deployment: Fields,
    deployment_secret_access_key: bool,
    /// Where the deployment's values come from: `Helm values` or `.env`.
    deployment_source: &'static str,
    defaults: Defaults,
    /// A secret can be saved only with a credential key to encrypt it.
    can_save_secret: bool,
    running: bool,
    backups: Vec<BackupSummary>,
    /// Workspaces the last restore put back, or could not.
    restores: Vec<WorkspaceRestore>,
    key: KeyView,
}

async fn administrator(api: &Api, headers: &HeaderMap) -> Result<Uuid, IdentityError> {
    let actor = super::identity::actor(&api.identity, headers).await?;
    if !actor.administrator {
        return Err(IdentityError::Forbidden);
    }
    Ok(actor.id)
}

fn configured(backups: Option<Extension<Backups>>) -> Result<Backups, IdentityError> {
    backups
        .map(|Extension(backups)| backups)
        .ok_or(IdentityError::NotFound)
}

async fn view(api: &Api, backups: &Backups, saved: Saved) -> Result<BackupView, IdentityError> {
    let (deployment, deployment_secret) = Fields::deployment();
    let key = sqlx::query!(
        r#"
        SELECT EXTRACT(EPOCH FROM s.key_downloaded_at)::bigint AS at, a.login AS "login?"
        FROM backup_settings s LEFT JOIN human_accounts a ON a.id=s.key_downloaded_by
        "#
    )
    .fetch_one(&api.pool)
    .await?;
    let kubernetes = std::env::var_os("CUBEBOT_CONTROLLER_CONFIG").is_some();
    Ok(BackupView {
        secret_access_key_saved: saved.secret.is_some(),
        settings: saved.fields,
        revision: saved.revision,
        deployment,
        deployment_secret_access_key: deployment_secret.is_some(),
        deployment_source: if kubernetes { "Helm values" } else { ".env" },
        defaults: Defaults {
            region: "us-east-1",
            schedule: "daily",
            keep_daily: settings::KEEP_DAILY,
            keep_weekly: settings::KEEP_WEEKLY,
            bandwidth_bytes: settings::BANDWIDTH,
            concurrency: settings::CONCURRENCY,
        },
        can_save_secret: backups.credentials().is_some(),
        running: backups.running(),
        backups: backups.history().await?,
        restores: backups.workspace_restores().await?,
        key: KeyView {
            present: backups.credential_key().is_some(),
            downloaded_at_unix: key.at,
            downloaded_by: key.login,
        },
    })
}

/// Administrators only.
#[utoipa::path(get, path = "/api/installation/backup", responses((status = 200, body = BackupView), (status = 404)))]
async fn backup(
    State(api): State<Api>,
    headers: HeaderMap,
    backups: Option<Extension<Backups>>,
) -> Result<impl IntoResponse, IdentityError> {
    administrator(&api, &headers).await?;
    let backups = configured(backups)?;
    let saved = Saved::load(&api.pool).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(view(&api, &backups, saved).await?),
    ))
}

/// Administrators only. Values the deployment sets are kept whatever is sent. 409 when
/// `revision` is not the current one.
#[utoipa::path(put, path = "/api/installation/backup", request_body = SaveBackup, responses((status = 200, body = BackupView), (status = 400), (status = 409)))]
async fn save_backup(
    State(api): State<Api>,
    headers: HeaderMap,
    backups: Option<Extension<Backups>>,
    Json(input): Json<SaveBackup>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = administrator(&api, &headers).await?;
    let backups = configured(backups)?;
    let saved = settings::save(&api.pool, backups.credentials(), &input).await?;
    let detail = serde_json::json!({
        "bucket": saved.fields.bucket,
        "endpoint": saved.fields.endpoint,
        "schedule": saved.fields.schedule,
        "secret_access_key_changed": input.secret_access_key.is_some(),
    });
    api.identity
        .audit(Event::by(actor, "backup.settings").detail(detail))
        .await;
    Ok((
        [("cache-control", "no-store")],
        Json(view(&api, &backups, saved).await?),
    ))
}

/// Administrators only. Starts a backup; 409 while one runs, 400 without a bucket.
#[utoipa::path(post, path = "/api/installation/backup/run", responses((status = 202), (status = 400), (status = 409)))]
async fn back_up_now(
    State(api): State<Api>,
    headers: HeaderMap,
    backups: Option<Extension<Backups>>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = administrator(&api, &headers).await?;
    let backups = configured(backups)?;
    let config = Saved::load(&api.pool).await?.config(backups.credentials());
    if config.target.is_none() {
        return Err(IdentityError::InvalidInput);
    }
    backups.request(actor)?;
    api.identity.audit(Event::by(actor, "backup.started")).await;
    Ok(StatusCode::ACCEPTED)
}

/// Administrators only, once per installation: the credential key, base64 encoded, as
/// `cubebot key` prints it. 409 once it was downloaded; 404 without a key.
#[utoipa::path(post, path = "/api/installation/backup/key", responses((status = 200, content_type = "text/plain", body = String), (status = 404), (status = 409)))]
async fn download_key(
    State(api): State<Api>,
    headers: HeaderMap,
    backups: Option<Extension<Backups>>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = administrator(&api, &headers).await?;
    let key = configured(backups)?
        .credential_key()
        .ok_or(IdentityError::NotFound)?;
    let claimed = sqlx::query!(
        "
        UPDATE backup_settings SET key_downloaded_at=clock_timestamp(), key_downloaded_by=$1
        WHERE key_downloaded_at IS NULL
        ",
        actor,
    )
    .execute(&api.pool)
    .await?;
    if claimed.rows_affected() == 0 {
        return Err(IdentityError::Conflict);
    }
    api.identity
        .audit(Event::by(actor, "backup.key_downloaded"))
        .await;
    Ok((
        [
            ("cache-control", "no-store"),
            ("content-type", "text/plain; charset=utf-8"),
            (
                "content-disposition",
                "attachment; filename=\"cubebot-credential-key.txt\"",
            ),
        ],
        format!("{}\n", base64(&key)),
    ))
}

/// Standard base64 with padding, as `base64 -d` reads it.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut text = String::new();
    for group in bytes.chunks(3) {
        let joined = group
            .iter()
            .fold(0u32, |joined, byte| joined << 8 | u32::from(*byte))
            << (8 * (3 - group.len()));
        for index in 0..4 {
            let sextet = (joined >> (18 - 6 * index)) & 63;
            text.push(if index <= group.len() {
                char::from(ALPHABET[sextet as usize])
            } else {
                '='
            });
        }
    }
    text
}

/// Whether a restore is running. The console asks before anything else; while a restore runs
/// this is the only route that answers.
#[utoipa::path(get, path = "/api/restore", responses((status = 200, body = RestoreProgress)))]
async fn restore_progress() -> impl IntoResponse {
    (
        [("cache-control", "no-store")],
        Json(RestoreProgress {
            state: "none",
            ..RestoreProgress::default()
        }),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_matches_the_standard_alphabet_and_padding() {
        assert_eq!(super::base64(b""), "");
        assert_eq!(super::base64(b"f"), "Zg==");
        assert_eq!(super::base64(b"fo"), "Zm8=");
        assert_eq!(super::base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(super::base64(&[255; 32]).len(), 44);
    }
}
