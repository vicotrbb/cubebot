//! A download is always an attachment the browser never renders in the console's origin.
use crate::{
    identity::{Identity, IdentityError},
    storage::{Principal, Storage, StorageError, Upload},
};
use axum::{
    Extension, Json, Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
};
use cubebot_protocol::runtime::{FileEntry, FileShare};
use futures_util::Stream;
use serde::Deserialize;
use std::io::SeekFrom;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeekExt};
use uuid::Uuid;

#[derive(Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(deny_unknown_fields)]
pub(super) struct Folder {
    /// The folder; your own top level when absent.
    parent: Option<Uuid>,
}

#[derive(Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(deny_unknown_fields)]
pub(super) struct UploadQuery {
    /// The folder; your own top level when absent.
    pub parent: Option<Uuid>,
    pub name: String,
    /// Your id for this upload: a retry with it returns the file the first attempt made.
    pub request_id: Uuid,
}

/// A new folder, or where a file goes: `parent` null or absent is the owner's top level.
#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct Placement {
    parent: Option<Uuid>,
    name: String,
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(
    list,
    upload,
    create_folder,
    shared,
    place,
    delete,
    content,
    shares,
    replace_shares
))]
pub(super) struct Doc;

pub(super) fn router(identity: Identity) -> Router {
    Router::new()
        .route("/api/files", get(list).post(upload))
        .route("/api/files/folders", post(create_folder))
        .route("/api/files/shared", get(shared))
        .route("/api/files/{file}", patch(place).delete(delete))
        .route("/api/files/{file}/content", get(content))
        .route("/api/files/{file}/shares", get(shares).put(replace_shares))
        .layer(DefaultBodyLimit::max(64 * 1024))
        .with_state(identity)
}

pub(super) enum Failure {
    Identity(IdentityError),
    Storage(StorageError),
}

impl From<IdentityError> for Failure {
    fn from(error: IdentityError) -> Self {
        Self::Identity(error)
    }
}

impl From<StorageError> for Failure {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        match self {
            Self::Identity(error) => error.into_response(),
            Self::Storage(error) => refused(error),
        }
    }
}

pub(super) fn refused(error: StorageError) -> Response {
    let (status, reason) = status(&error);
    log(&error, status);
    let body = Json(serde_json::json!({ "error": reason }));
    let mut response = (status, [(header::CACHE_CONTROL, "no-store")], body).into_response();
    if status == StatusCode::TOO_MANY_REQUESTS {
        let retry = HeaderValue::from_static("1");
        response.headers_mut().insert(header::RETRY_AFTER, retry);
    }
    response
}

fn log(error: &StorageError, status: StatusCode) {
    if status.is_server_error() || matches!(error, StorageError::Incomplete(_)) {
        tracing::warn!(?error, "shared file request failed");
    }
}

fn status(error: &StorageError) -> (StatusCode, &'static str) {
    match error {
        StorageError::Invalid(reason) => (StatusCode::BAD_REQUEST, reason),
        StorageError::Incomplete(_) => (StatusCode::BAD_REQUEST, "upload_incomplete"),
        StorageError::Forbidden => (StatusCode::FORBIDDEN, "forbidden"),
        StorageError::NotFound => (StatusCode::NOT_FOUND, "not_found"),
        StorageError::Conflict => (StatusCode::CONFLICT, "name_taken"),
        StorageError::Busy => (StatusCode::TOO_MANY_REQUESTS, "storage_busy"),
        StorageError::Full => (StatusCode::INSUFFICIENT_STORAGE, "storage_full"),
        StorageError::TooLarge => (StatusCode::PAYLOAD_TOO_LARGE, "file_too_large"),
        StorageError::Integrity(..) => (StatusCode::INTERNAL_SERVER_ERROR, "integrity_fault"),
        StorageError::Unavailable | StorageError::Io(_) | StorageError::Database(_) => {
            (StatusCode::SERVICE_UNAVAILABLE, "storage_unavailable")
        }
    }
}

pub(super) fn configured(storage: Option<Extension<Storage>>) -> Result<Storage, StorageError> {
    storage
        .map(|Extension(storage)| storage)
        .ok_or(StorageError::Unavailable)
}

async fn person(
    identity: &Identity,
    headers: &HeaderMap,
    storage: Option<Extension<Storage>>,
) -> Result<(Storage, Principal), Failure> {
    let actor = super::identity::actor(identity, headers).await?;
    Ok((configured(storage)?, Principal::Person(actor.id)))
}

pub(super) async fn store(
    storage: &Storage,
    who: Principal,
    query: UploadQuery,
    headers: &HeaderMap,
    body: Body,
) -> Result<Response, StorageError> {
    let content_type = headers.get(header::CONTENT_TYPE);
    let upload = Upload {
        parent: query.parent,
        name: query.name,
        content_type: content_type
            .and_then(|value| value.to_str().ok())
            .map(Into::into),
        request: query.request_id,
    };
    let (entry, created) = storage
        .upload(who, &upload, body.into_data_stream())
        .await?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(entry)).into_response())
}

/// Your top level, or a folder you may read.
#[utoipa::path(get, path = "/api/files", operation_id = "list_files", params(Folder), responses((status = 200, body = Vec<FileEntry>)))]
async fn list(
    State(identity): State<Identity>,
    headers: HeaderMap,
    storage: Option<Extension<Storage>>,
    Query(folder): Query<Folder>,
) -> Result<Json<Vec<FileEntry>>, Failure> {
    let (storage, who) = person(&identity, &headers, storage).await?;
    Ok(Json(storage.list(who, folder.parent).await?))
}

/// The body is the file's bytes and its content type the file's; 507 when the installation's
/// quota is used up, 409 when the name is taken in the folder.
#[utoipa::path(post, path = "/api/files", operation_id = "upload_file", params(UploadQuery), request_body(content = Vec<u8>, content_type = "application/octet-stream"), responses((status = 201, body = FileEntry), (status = 200, body = FileEntry)))]
async fn upload(
    State(identity): State<Identity>,
    headers: HeaderMap,
    storage: Option<Extension<Storage>>,
    Query(query): Query<UploadQuery>,
    body: Body,
) -> Result<Response, Failure> {
    let (storage, who) = person(&identity, &headers, storage).await?;
    Ok(store(&storage, who, query, &headers, body).await?)
}

#[utoipa::path(post, path = "/api/files/folders", operation_id = "create_folder", request_body = Placement, responses((status = 201, body = FileEntry)))]
async fn create_folder(
    State(identity): State<Identity>,
    headers: HeaderMap,
    storage: Option<Extension<Storage>>,
    Json(input): Json<Placement>,
) -> Result<(StatusCode, Json<FileEntry>), Failure> {
    let (storage, who) = person(&identity, &headers, storage).await?;
    let folder = storage
        .create_folder(who, input.parent, &input.name)
        .await?;
    Ok((StatusCode::CREATED, Json(folder)))
}

/// Files and folders shared with you or your teams.
#[utoipa::path(get, path = "/api/files/shared", operation_id = "shared_files", responses((status = 200, body = Vec<FileEntry>)))]
async fn shared(
    State(identity): State<Identity>,
    headers: HeaderMap,
    storage: Option<Extension<Storage>>,
) -> Result<Json<Vec<FileEntry>>, Failure> {
    let (storage, who) = person(&identity, &headers, storage).await?;
    Ok(Json(storage.shared(who).await?))
}

/// Renames and moves within the owner's folders.
#[utoipa::path(patch, path = "/api/files/{file}", operation_id = "place_file", params(("file" = Uuid, Path)), request_body = Placement, responses((status = 200, body = FileEntry)))]
async fn place(
    State(identity): State<Identity>,
    headers: HeaderMap,
    storage: Option<Extension<Storage>>,
    Path(file): Path<Uuid>,
    Json(input): Json<Placement>,
) -> Result<Json<FileEntry>, Failure> {
    let (storage, who) = person(&identity, &headers, storage).await?;
    Ok(Json(
        storage.place(who, file, input.parent, &input.name).await?,
    ))
}

/// Deletes a file, or a folder and everything under it.
#[utoipa::path(delete, path = "/api/files/{file}", operation_id = "delete_file", params(("file" = Uuid, Path)), responses((status = 204)))]
async fn delete(
    State(identity): State<Identity>,
    headers: HeaderMap,
    storage: Option<Extension<Storage>>,
    Path(file): Path<Uuid>,
) -> Result<StatusCode, Failure> {
    let (storage, who) = person(&identity, &headers, storage).await?;
    storage.delete(who, file).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The file's bytes as an attachment; one `Range` of bytes gives 206.
#[utoipa::path(get, path = "/api/files/{file}/content", operation_id = "file_content", params(("file" = Uuid, Path)), responses((status = 200, body = Vec<u8>, content_type = "application/octet-stream"), (status = 206), (status = 416)))]
async fn content(
    State(identity): State<Identity>,
    headers: HeaderMap,
    storage: Option<Extension<Storage>>,
    Path(file): Path<Uuid>,
) -> Result<Response, Failure> {
    let (storage, who) = person(&identity, &headers, storage).await?;
    let (entry, bytes) = storage.open(who, file).await?;
    Ok(download(entry, bytes, &headers).await)
}

/// The file's own shares; inherited ones are on the folders above it.
#[utoipa::path(get, path = "/api/files/{file}/shares", operation_id = "file_shares", params(("file" = Uuid, Path)), responses((status = 200, body = Vec<FileShare>)))]
async fn shares(
    State(identity): State<Identity>,
    headers: HeaderMap,
    storage: Option<Extension<Storage>>,
    Path(file): Path<Uuid>,
) -> Result<Json<Vec<FileShare>>, Failure> {
    let (storage, who) = person(&identity, &headers, storage).await?;
    Ok(Json(storage.shares(who, file).await?))
}

/// Replaces the file's own shares.
#[utoipa::path(put, path = "/api/files/{file}/shares", operation_id = "replace_file_shares", params(("file" = Uuid, Path)), request_body = Vec<FileShare>, responses((status = 200, body = Vec<FileShare>)))]
async fn replace_shares(
    State(identity): State<Identity>,
    headers: HeaderMap,
    storage: Option<Extension<Storage>>,
    Path(file): Path<Uuid>,
    Json(input): Json<Vec<FileShare>>,
) -> Result<Json<Vec<FileShare>>, Failure> {
    let (storage, who) = person(&identity, &headers, storage).await?;
    Ok(Json(storage.share(who, file, &input, true).await?))
}

pub(super) async fn download(
    entry: FileEntry,
    mut bytes: tokio::fs::File,
    headers: &HeaderMap,
) -> Response {
    let size = entry.size.unwrap_or_default() as u64;
    let range = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok());
    let Ok(range) = byte_range(range, size) else {
        let unsatisfied = [(header::CONTENT_RANGE, format!("bytes */{size}"))];
        return (StatusCode::RANGE_NOT_SATISFIABLE, unsatisfied).into_response();
    };
    let (start, end) = range.unwrap_or((0, size));
    if let Err(error) = bytes.seek(SeekFrom::Start(start)).await {
        return refused(StorageError::Io(error));
    }
    let content_type = entry.content_type.as_deref();
    let mut response = Response::builder()
        .header(
            header::CONTENT_TYPE,
            content_type.unwrap_or("application/octet-stream"),
        )
        .header(header::CONTENT_LENGTH, end - start)
        .header(header::CONTENT_DISPOSITION, disposition(&entry.name))
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::CONTENT_SECURITY_POLICY, "sandbox")
        .header(header::CACHE_CONTROL, "no-store");
    if range.is_some() {
        let served = format!("bytes {start}-{}/{size}", end - 1);
        response = response
            .status(StatusCode::PARTIAL_CONTENT)
            .header(header::CONTENT_RANGE, served);
    }
    let body = Body::from_stream(chunks(bytes.take(end - start)));
    response
        .body(body)
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn chunks(
    reader: impl AsyncRead + Unpin + Send + 'static,
) -> impl Stream<Item = std::io::Result<Bytes>> + Send + 'static {
    futures_util::stream::unfold(Some(reader), |state| async move {
        let mut reader = state?;
        let mut buffer = vec![0; 64 * 1024];
        match reader.read(&mut buffer).await {
            Ok(0) => None,
            Ok(read) => {
                buffer.truncate(read);
                Some((Ok(Bytes::from(buffer)), Some(reader)))
            }
            Err(error) => Some((Err(error), None)),
        }
    })
}

fn byte_range(header: Option<&str>, size: u64) -> Result<Option<(u64, u64)>, ()> {
    let Some((first, last)) = header
        .and_then(|value| value.strip_prefix("bytes="))
        .and_then(|value| value.split_once('-'))
    else {
        return Ok(None);
    };
    let number = |text: &str| (!text.is_empty()).then(|| text.parse::<u64>()).transpose();
    let (Ok(first), Ok(last)) = (number(first), number(last)) else {
        return Ok(None);
    };
    match (first, last) {
        (Some(start), Some(last)) if last < start => Ok(None),
        (Some(start), _) if start >= size => Err(()),
        (Some(start), last) => Ok(Some((
            start,
            last.map_or(size, |last| last.saturating_add(1).min(size)),
        ))),
        (None, Some(suffix)) if suffix > 0 && size > 0 => Ok(Some((size - suffix.min(size), size))),
        (None, Some(_)) => Err(()),
        (None, None) => Ok(None),
    }
}

fn disposition(name: &str) -> String {
    let fallback: String = name
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '-' | '_' | ' ' => c,
            _ => '_',
        })
        .collect();
    let encoded: String = name
        .bytes()
        .map(|byte| match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'-' | b'_' | b'~' => {
                char::from(byte).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect();
    format!("attachment; filename=\"{fallback}\"; filename*=UTF-8''{encoded}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_satisfiable_range_is_served() {
        let range = |header| byte_range(Some(header), 100);
        assert_eq!(byte_range(None, 100), Ok(None));
        assert_eq!(range("bytes=0-9"), Ok(Some((0, 10))));
        assert_eq!(range("bytes=90-"), Ok(Some((90, 100))));
        assert_eq!(range("bytes=95-500"), Ok(Some((95, 100))));
        assert_eq!(range("bytes=-10"), Ok(Some((90, 100))));
        assert_eq!(range("bytes=-500"), Ok(Some((0, 100))));
        assert_eq!(range("bytes=100-"), Err(()));
        assert_eq!(range("bytes=-0"), Err(()));
        assert_eq!(byte_range(Some("bytes=0-"), 0), Err(()));
        assert_eq!(range("bytes=9-0"), Ok(None));
        assert_eq!(range("bytes=0-1,5-6"), Ok(None));
        assert_eq!(range("items=0-1"), Ok(None));
        assert_eq!(range("bytes=x-1"), Ok(None));
    }

    #[test]
    fn a_download_name_cannot_break_the_header() {
        assert_eq!(
            disposition("a\"b\r\nc é.txt"),
            "attachment; filename=\"a_b__c _.txt\"; filename*=UTF-8''a%22b%0D%0Ac%20%C3%A9.txt"
        );
    }
}
