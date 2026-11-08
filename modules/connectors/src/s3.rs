//! S3-compatible object storage (https://docs.aws.amazon.com/AmazonS3/latest/API/): AWS S3,
//! Cloudflare R2, Backblaze B2, MinIO and others. Requests are signed with AWS Signature Version 4
//! (https://docs.aws.amazon.com/IAM/latest/UserGuide/reference_sigv-create-signed-request.html)
//! and sent through the shared HTTP client, so the AWS SDK is not needed.
use crate::{
    Account, Answer, Attachment, Auth, Category, Choice, Connector, Context, Error, Field,
    FieldKind, Level, Manifest, PRIVATE_ADDRESS, Risk, Step, ToolSpec,
};
use hmac::{Hmac, Mac};
use reqwest::{Method, RequestBuilder, Url};
use roxmltree::{Document, Node};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const LISTED: u32 = 100;
const MOST_LISTED: u32 = 1000;
/// SHA-256 of nothing, the payload hash of a request without a body.
const EMPTY: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

pub struct S3;

static MANIFEST: Manifest = Manifest {
    id: "s3",
    name: "S3-compatible storage",
    description: "List, read, upload, copy and delete objects in AWS S3, Cloudflare R2, Backblaze B2 or MinIO.",
    category: Category::Data,
    auth: &[Auth::ApiKey {
        fields: &[
            Field {
                key: "access_key_id",
                label: "Access key ID",
                kind: FieldKind::Text,
                required: true,
                placeholder: "AKIAIOSFODNN7EXAMPLE",
                help: "",
            },
            Field {
                key: "secret_access_key",
                label: "Secret access key",
                kind: FieldKind::Secret,
                required: true,
                placeholder: "",
                help: "Shown once, when the key is made.",
            },
        ],
    }],
    config: &[
        Field {
            key: "endpoint",
            label: "Endpoint",
            kind: FieldKind::Url,
            required: false,
            placeholder: "https://<account>.r2.cloudflarestorage.com",
            help: "Leave empty for AWS S3. Backblaze B2: https://s3.<region>.backblazeb2.com; MinIO: its API address, such as http://minio.local:9000.",
        },
        Field {
            key: "region",
            label: "Region",
            kind: FieldKind::Text,
            required: false,
            placeholder: "us-east-1",
            help: "us-east-1 when empty; auto for Cloudflare R2.",
        },
        Field {
            key: "bucket",
            label: "Default bucket",
            kind: FieldKind::Text,
            required: false,
            placeholder: "",
            help: "Used when a tool names no bucket, and to test the connection.",
        },
        Field {
            key: "addressing",
            label: "Addressing",
            kind: FieldKind::Choice(&[
                Choice {
                    value: "path",
                    label: "Path style: endpoint/bucket/key, as MinIO needs",
                },
                Choice {
                    value: "virtual",
                    label: "Virtual-hosted: bucket.endpoint/key",
                },
            ]),
            required: false,
            placeholder: "",
            help: "When not set: virtual-hosted for AWS, path style for any other endpoint.",
        },
    ],
    products: &[],
    tools: &[
        ToolSpec {
            name: "list_buckets",
            description: "The buckets the key can see, with when each was made.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "list_objects",
            description: "Objects in a bucket, optionally under a prefix. With delimiter / the folders under the prefix are listed apart. Pages of up to 1000; pass continuation from the last page.",
            risk: Risk::Read,
            product: None,
            input: concat!(
                r#"{"type":"object","properties":{"#,
                r#""bucket":{"type":"string","description":"Leave out for the connection's default bucket."}"#,
                r#","prefix":{"type":"string","description":"Such as reports/2026/."},"delimiter":{"type":"string","description":"Usually /."},"continuation":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":1000}},"additionalProperties":false}"#
            ),
        },
        ToolSpec {
            name: "head_object",
            description: "An object's size, type, modification time, ETag and metadata, without its content.",
            risk: Risk::Read,
            product: None,
            input: concat!(
                r#"{"type":"object","properties":{"#,
                r#""bucket":{"type":"string","description":"Leave out for the connection's default bucket."}"#,
                r#","key":{"type":"string"}},"required":["key"],"additionalProperties":false}"#
            ),
        },
        ToolSpec {
            name: "get_object",
            description: "Save an object to the workspace, up to 25 MiB.",
            risk: Risk::Read,
            product: None,
            input: concat!(
                r#"{"type":"object","properties":{"#,
                r#""bucket":{"type":"string","description":"Leave out for the connection's default bucket."}"#,
                r#","key":{"type":"string"}},"required":["key"],"additionalProperties":false}"#
            ),
        },
        ToolSpec {
            name: "put_object",
            description: "Upload a workspace file as an object, replacing any object at that key.",
            risk: Risk::Write,
            product: None,
            input: concat!(
                r#"{"type":"object","properties":{"#,
                r#""bucket":{"type":"string","description":"Leave out for the connection's default bucket."}"#,
                r#","key":{"type":"string","description":"Where to store it; the file's name when left out."},"workspace_file":{"type":"string","description":"A file in your workspace."},"content_type":{"type":"string","description":"Such as text/csv."}},"required":["workspace_file"],"additionalProperties":false}"#
            ),
        },
        ToolSpec {
            name: "delete_object",
            description: "Delete an object. Without versioning it cannot be recovered.",
            risk: Risk::Write,
            product: None,
            input: concat!(
                r#"{"type":"object","properties":{"#,
                r#""bucket":{"type":"string","description":"Leave out for the connection's default bucket."}"#,
                r#","key":{"type":"string"}},"required":["key"],"additionalProperties":false}"#
            ),
        },
        ToolSpec {
            name: "copy_object",
            description: "Copy an object to a new key, in the same bucket or another one on this endpoint.",
            risk: Risk::Write,
            product: None,
            input: concat!(
                r#"{"type":"object","properties":{"#,
                r#""bucket":{"type":"string","description":"Leave out for the connection's default bucket."}"#,
                r#","key":{"type":"string","description":"The copy's key."},"source_bucket":{"type":"string","description":"The default bucket when left out."},"source_key":{"type":"string"}},"required":["key","source_key"],"additionalProperties":false}"#
            ),
        },
    ],
    events: &[],
    level: Level::Read,
    guide: &[
        Step {
            title: "Make an access key",
            body: "AWS: in IAM create a user with a policy limited to the buckets agents need, then under Security credentials create an access key. Cloudflare R2: under R2, Manage API tokens, create an Object Read or Read & Write token and copy its S3 access key ID and secret. Backblaze B2: under Application Keys add a key limited to one bucket. MinIO: under Access Keys create one with a policy for the buckets.",
            link: None,
            copy: None,
        },
        Step {
            title: "Enter the endpoint and region",
            body: "Leave the endpoint empty for AWS and give the bucket's region. For R2 use https://<account id>.r2.cloudflarestorage.com and region auto; for B2 the S3 endpoint on the bucket's page and its region, such as us-west-004; for MinIO its API address (port 9000 by default, not the console's) with path-style addressing.",
            link: None,
            copy: None,
        },
        PRIVATE_ADDRESS,
    ],
    warning: Some(
        "The key acts with every permission its policy grants, whatever level this connection has. Limit it to the buckets agents need, and to reading unless they should write.",
    ),
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Nothing {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Listing {
    bucket: Option<String>,
    prefix: Option<String>,
    delimiter: Option<String>,
    continuation: Option<String>,
    limit: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Object {
    bucket: Option<String>,
    key: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Put {
    bucket: Option<String>,
    key: Option<String>,
    workspace_file: Attachment,
    content_type: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Copy {
    bucket: Option<String>,
    key: String,
    source_bucket: Option<String>,
    source_key: String,
}

impl Connector for S3 {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn check<'a>(&'a self, cx: &'a Context) -> Answer<'a, Account> {
        Box::pin(async move {
            if let Some(bucket) = cx.setting("bucket") {
                let bucket = checked_bucket(bucket)?;
                cx.send(signed(cx, Request::new(Method::HEAD, Some(bucket), ""))?)
                    .await
                    .map_err(tidy)?;
                return Ok(Account {
                    name: bucket.into(),
                });
            }
            let listed = buckets(cx).await?;
            let owner = listed["owner"].as_str().filter(|owner| !owner.is_empty());
            let name = owner.or(cx.setting("access_key_id")).unwrap_or("S3");
            Ok(Account { name: name.into() })
        })
    }

    fn call<'a>(&'a self, cx: &'a Context, tool: &'a str, input: Value) -> Answer<'a, Value> {
        Box::pin(async move {
            match tool {
                "list_buckets" => {
                    crate::input::<Nothing>(input)?;
                    buckets(cx).await
                }
                "list_objects" => objects(cx, input).await,
                "head_object" => head(cx, input).await,
                "get_object" => get(cx, input).await,
                "put_object" => put(cx, input).await,
                "delete_object" => delete(cx, input).await,
                "copy_object" => copy(cx, input).await,
                _ => Err(Error::UnknownTool),
            }
        })
    }
}

/// One S3 request before signing. An empty key with a bucket addresses the bucket itself.
struct Request<'a> {
    method: Method,
    bucket: Option<&'a str>,
    key: &'a str,
    query: Vec<(&'static str, String)>,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
}

impl<'a> Request<'a> {
    fn new(method: Method, bucket: Option<&'a str>, key: &'a str) -> Self {
        Self {
            method,
            bucket,
            key,
            query: Vec::new(),
            headers: Vec::new(),
            body: Vec::new(),
        }
    }
}

/// What a request is signed with; `date` is `yyyymmddThhmmssZ`.
struct Signing<'a> {
    key_id: &'a str,
    secret: &'a str,
    region: &'a str,
    service: &'a str,
    date: &'a str,
}

/// A request in the form it is signed in: a percent-encoded path and query, and its headers.
struct Canonical<'a> {
    method: &'a str,
    path: &'a str,
    query: &'a str,
    headers: Vec<(&'a str, &'a str)>,
    payload: &'a str,
}

/// The `Authorization` header of a request signed with Signature Version 4.
fn authorization(signing: &Signing, request: &Canonical) -> String {
    let mut headers = request.headers.clone();
    headers.sort();
    let listed: String = headers
        .iter()
        .map(|(name, value)| format!("{name}:{}\n", value.trim()))
        .collect();
    let signed = headers
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(";");
    let canonical = format!(
        "{}\n{}\n{}\n{listed}\n{signed}\n{}",
        request.method, request.path, request.query, request.payload
    );
    let day = &signing.date[..8];
    let scope = format!("{day}/{}/{}/aws4_request", signing.region, signing.service);
    let to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{scope}\n{}",
        signing.date,
        hex(&Sha256::digest(canonical.as_bytes()))
    );
    let key = [signing.region, signing.service, "aws4_request"]
        .into_iter()
        .fold(
            mac(format!("AWS4{}", signing.secret).as_bytes(), day),
            |key, part| mac(&key, part),
        );
    format!(
        "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed}, Signature={}",
        signing.key_id,
        hex(&mac(&key, &to_sign))
    )
}

fn mac(key: &[u8], text: &str) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes a key of any length");
    mac.update(text.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// RFC 3986 percent-encoding of all but unreserved characters, keeping `/` in a path.
fn encode(text: &str, path: bool) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                char::from(byte).to_string()
            }
            b'/' if path => "/".into(),
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

/// The sorted, encoded query string the signature and the URL share.
fn query(pairs: &[(&str, String)]) -> String {
    let mut encoded: Vec<String> = pairs
        .iter()
        .map(|(name, value)| format!("{}={}", encode(name, false), encode(value, false)))
        .collect();
    encoded.sort();
    encoded.join("&")
}

fn amz_date() -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
    let at = mail_parser::DateTime::from_timestamp(now.map_or(0, |now| now.as_secs() as i64));
    format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
        at.year, at.month, at.day, at.hour, at.minute, at.second
    )
}

fn region(cx: &Context) -> Result<&str, Error> {
    checked_region(cx.setting("region").unwrap_or("us-east-1").trim())
}

pub fn checked_region(region: &str) -> Result<&str, Error> {
    let valid = !region.is_empty()
        && region
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    valid
        .then_some(region)
        .ok_or_else(|| Error::Invalid("the region is letters, digits and hyphens".into()))
}

/// The endpoint and whether buckets go in its path.
fn endpoint(
    custom: Option<&str>,
    addressing: Option<&str>,
    region: &str,
) -> Result<(Url, bool), Error> {
    let (address, custom) = match custom {
        Some(endpoint) => (endpoint.trim().to_owned(), true),
        None => (format!("https://s3.{region}.amazonaws.com"), false),
    };
    let url = Url::parse(&address)
        .ok()
        .filter(|url| url.host_str().is_some())
        .ok_or_else(|| Error::Invalid("the endpoint is not a valid address".into()))?;
    let path_style = match addressing {
        Some("path") => true,
        Some("virtual") => false,
        _ => custom,
    };
    Ok((url, path_style))
}

fn checked_bucket(bucket: &str) -> Result<&str, Error> {
    let valid = (3..=63).contains(&bucket.len())
        && bucket
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'));
    valid
        .then_some(bucket)
        .ok_or_else(|| Error::Invalid(format!("{bucket:?} is not a bucket name")))
}

fn bucket<'a>(cx: &'a Context, named: Option<&'a str>) -> Result<&'a str, Error> {
    let bucket = named
        .filter(|bucket| !bucket.is_empty())
        .or(cx.setting("bucket"))
        .ok_or_else(|| {
            Error::Invalid("name a bucket: the connection has no default bucket".into())
        })?;
    checked_bucket(bucket)
}

/// Keys with `.` or `..` segments would be rewritten by URL normalisation, reaching another object.
fn checked_key(key: &str) -> Result<&str, Error> {
    let dotted = key
        .split('/')
        .any(|segment| segment == "." || segment == "..");
    if key.is_empty() || key.len() > 1024 || dotted {
        return Err(Error::Invalid(
            "a key is 1 to 1024 bytes without . or .. segments".into(),
        ));
    }
    Ok(key)
}

/// Signs requests for one endpoint, for callers that send them on their own client, such as
/// Cubebot's backups.
pub struct Signer<'a> {
    /// AWS S3 in `region` when absent.
    pub endpoint: Option<&'a str>,
    /// `path` or `virtual`; when absent, virtual-hosted for AWS and path style for any other.
    pub addressing: Option<&'a str>,
    pub region: &'a str,
    pub key_id: &'a str,
    pub secret: &'a str,
}

/// A signed request: its URL and every header to send, except `host`, which the client sets.
pub struct Signed {
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
}

/// The payload hash of a body sent without hashing it first, which S3 accepts over HTTPS and HTTP.
pub const UNSIGNED: &str = "UNSIGNED-PAYLOAD";

impl Signer<'_> {
    /// `payload` is the body's SHA-256 in lowercase hex, or [`UNSIGNED`]. An empty key with a
    /// bucket addresses the bucket itself.
    pub fn sign(
        &self,
        method: &Method,
        bucket: Option<&str>,
        key: &str,
        query: &[(&'static str, String)],
        headers: Vec<(&'static str, String)>,
        payload: &str,
    ) -> Result<Signed, Error> {
        let region = checked_region(self.region)?;
        let (endpoint, path_style) = endpoint(self.endpoint, self.addressing, region)?;
        let host = endpoint.host_str().unwrap_or_default();
        let port = endpoint.port().map(|port| format!(":{port}"));
        let key = encode(key, true);
        let (host, path) = match (bucket, path_style) {
            (None, _) => (host.to_owned(), "/".to_owned()),
            (Some(bucket), true) if key.is_empty() => (host.to_owned(), format!("/{bucket}")),
            (Some(bucket), true) => (host.to_owned(), format!("/{bucket}/{key}")),
            (Some(bucket), false) => (format!("{bucket}.{host}"), format!("/{key}")),
        };
        let host = format!("{host}{}", port.unwrap_or_default());
        let query = self::query(query);
        let mut url = format!("{}://{host}{path}", endpoint.scheme());
        if !query.is_empty() {
            url = format!("{url}?{query}");
        }
        let date = amz_date();
        let mut headers = headers;
        headers.extend([
            ("x-amz-content-sha256", payload.to_owned()),
            ("x-amz-date", date.clone()),
        ]);
        let mut signed: Vec<(&str, &str)> = vec![("host", &host)];
        signed.extend(headers.iter().map(|(name, value)| (*name, value.as_str())));
        let signing = Signing {
            key_id: self.key_id.trim(),
            secret: self.secret,
            region,
            service: "s3",
            date: &date,
        };
        let canonical = Canonical {
            method: method.as_str(),
            path: &path,
            query: &query,
            headers: signed,
            payload,
        };
        let authorization = authorization(&signing, &canonical);
        headers.push(("authorization", authorization));
        Ok(Signed { url, headers })
    }
}

/// The signed request, its URL built for the endpoint's addressing style.
fn signed(cx: &Context, request: Request) -> Result<RequestBuilder, Error> {
    let signer = Signer {
        endpoint: cx.setting("endpoint"),
        addressing: cx.setting("addressing"),
        region: region(cx)?,
        key_id: cx.setting("access_key_id").ok_or(Error::Unauthorized)?,
        secret: cx.secret("secret_access_key")?,
    };
    let payload = match request.body.is_empty() {
        true => EMPTY.to_owned(),
        false => hex(&Sha256::digest(&request.body)),
    };
    let signed = signer.sign(
        &request.method,
        request.bucket,
        request.key,
        &request.query,
        request.headers,
        &payload,
    )?;
    let mut builder = cx.request(request.method, &signed.url)?;
    for (name, value) in signed.headers {
        builder = builder.header(name, value);
    }
    Ok(builder.body(request.body))
}

/// S3 answers errors in XML: its code and message replace the raw body, and a refused key is a
/// credential to sign in with again.
fn tidy(error: Error) -> Error {
    let between = |text: &str, tag: &str| {
        let start = text.find(&format!("<{tag}>"))? + tag.len() + 2;
        let rest = &text[start..];
        Some(rest[..rest.find('<').unwrap_or(rest.len())].to_owned())
    };
    let words = |text: String| match (between(&text, "Code"), between(&text, "Message")) {
        (Some(code), Some(message)) => format!("{code}: {message}"),
        (Some(code), None) => code,
        _ => text,
    };
    match error {
        Error::Forbidden(text)
            if ["InvalidAccessKeyId", "SignatureDoesNotMatch"]
                .iter()
                .any(|code| text.contains(code)) =>
        {
            Error::Unauthorized
        }
        Error::Forbidden(text) => Error::Forbidden(words(text)),
        Error::NotFound(text) => Error::NotFound(words(text)),
        Error::Rejected(text) => Error::Rejected(words(text)),
        Error::Unavailable(text) => Error::Unavailable(words(text)),
        other => other,
    }
}

fn xml(body: &[u8]) -> Result<Document<'_>, Error> {
    let text = std::str::from_utf8(body)
        .map_err(|_| Error::Unavailable("the answer was not UTF-8 XML".into()))?;
    Document::parse(text).map_err(|_| Error::Unavailable("the answer was not valid XML".into()))
}

fn child<'a>(node: Node<'a, 'a>, name: &str) -> Option<Node<'a, 'a>> {
    node.children()
        .find(|child| child.tag_name().name() == name)
}

fn text<'a>(node: Node<'a, 'a>, name: &str) -> Option<&'a str> {
    child(node, name).and_then(|child| child.text())
}

fn every<'a>(node: Node<'a, 'a>, name: &'a str) -> impl Iterator<Item = Node<'a, 'a>> + 'a {
    node.children()
        .filter(move |child| child.tag_name().name() == name)
}

async fn buckets(cx: &Context) -> Result<Value, Error> {
    let request = signed(cx, Request::new(Method::GET, None, ""))?;
    let (body, _) = cx.bytes(request).await.map_err(tidy)?;
    let document = xml(&body)?;
    let root = document.root_element();
    let listed: Vec<Value> = child(root, "Buckets")
        .into_iter()
        .flat_map(|buckets| every(buckets, "Bucket"))
        .map(
            |bucket| json!({"name": text(bucket, "Name"), "created": text(bucket, "CreationDate")}),
        )
        .collect();
    let owner = child(root, "Owner").and_then(|owner| text(owner, "DisplayName"));
    Ok(json!({"buckets": listed, "owner": owner}))
}

async fn objects(cx: &Context, input: Value) -> Result<Value, Error> {
    let listing: Listing = crate::input(input)?;
    let bucket = bucket(cx, listing.bucket.as_deref())?;
    let limit = listing.limit.unwrap_or(LISTED).clamp(1, MOST_LISTED);
    let mut request = Request::new(Method::GET, Some(bucket), "");
    request.query = vec![("list-type", "2".into()), ("max-keys", limit.to_string())];
    let optional = [
        ("prefix", listing.prefix),
        ("delimiter", listing.delimiter),
        ("continuation-token", listing.continuation),
    ];
    request.query.extend(
        optional
            .into_iter()
            .filter_map(|(name, value)| Some((name, value.filter(|value| !value.is_empty())?))),
    );
    let (body, _) = cx.bytes(signed(cx, request)?).await.map_err(tidy)?;
    let document = xml(&body)?;
    let root = document.root_element();
    let objects: Vec<Value> = every(root, "Contents")
        .map(|object| {
            json!({
                "key": text(object, "Key"),
                "size": text(object, "Size").and_then(|size| size.parse::<u64>().ok()),
                "modified": text(object, "LastModified"),
                "etag": text(object, "ETag"),
            })
        })
        .collect();
    let folders: Vec<Option<&str>> = every(root, "CommonPrefixes")
        .map(|folder| text(folder, "Prefix"))
        .collect();
    let truncated = text(root, "IsTruncated") == Some("true");
    let next = text(root, "NextContinuationToken").filter(|_| truncated);
    Ok(json!({"bucket": bucket, "objects": objects, "folders": folders, "continuation": next}))
}

/// One page of a ListObjectsV2 answer: each key with its size, and the next page's token.
pub struct Listed {
    pub objects: Vec<(String, u64)>,
    pub next: Option<String>,
}

pub fn listed(body: &[u8]) -> Result<Listed, Error> {
    let document = xml(body)?;
    let root = document.root_element();
    let objects = every(root, "Contents")
        .filter_map(|object| {
            let size = text(object, "Size")?.parse().ok()?;
            Some((text(object, "Key")?.to_owned(), size))
        })
        .collect();
    let truncated = text(root, "IsTruncated") == Some("true");
    let next = text(root, "NextContinuationToken").filter(|_| truncated);
    Ok(Listed {
        objects,
        next: next.map(str::to_owned),
    })
}

/// A field of an XML answer, such as a new multipart upload's `UploadId`. S3 may answer a
/// completed multipart upload with 200 and an `Error` document, which is refused here.
pub fn answered(body: &[u8], name: &str) -> Result<Option<String>, Error> {
    let document = xml(body)?;
    let root = document.root_element();
    if root.tag_name().name() == "Error" {
        let code = text(root, "Code").unwrap_or("Error");
        let message = text(root, "Message").unwrap_or_default();
        return Err(Error::Rejected(format!("{code}: {message}")));
    }
    Ok(text(root, name).map(str::to_owned))
}

async fn head(cx: &Context, input: Value) -> Result<Value, Error> {
    let object: Object = crate::input(input)?;
    let bucket = bucket(cx, object.bucket.as_deref())?;
    let key = checked_key(&object.key)?;
    let request = signed(cx, Request::new(Method::HEAD, Some(bucket), key))?;
    let response = cx.send(request).await.map_err(|error| match error {
        Error::NotFound(_) => Error::NotFound(format!("no object {key} in {bucket}")),
        other => tidy(other),
    })?;
    let headers = response.headers();
    let header = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
    let metadata: serde_json::Map<String, Value> = headers
        .iter()
        .filter_map(|(name, value)| {
            let name = name.as_str().strip_prefix("x-amz-meta-")?;
            Some((name.to_owned(), json!(value.to_str().ok()?)))
        })
        .collect();
    Ok(json!({
        "bucket": bucket,
        "key": key,
        "size": header("content-length").and_then(|size| size.parse::<u64>().ok()),
        "content_type": header("content-type"),
        "modified": header("last-modified"),
        "etag": header("etag"),
        "metadata": metadata,
    }))
}

async fn get(cx: &Context, input: Value) -> Result<Value, Error> {
    let object: Object = crate::input(input)?;
    let bucket = bucket(cx, object.bucket.as_deref())?;
    let key = checked_key(&object.key)?;
    let request = signed(cx, Request::new(Method::GET, Some(bucket), key))?;
    let (bytes, kind) = cx.bytes(request).await.map_err(tidy)?;
    let name = key.rsplit('/').next().filter(|name| !name.is_empty());
    Ok(crate::file(
        name.unwrap_or("object"),
        kind.as_deref(),
        &bytes,
    ))
}

async fn put(cx: &Context, input: Value) -> Result<Value, Error> {
    let put: Put = crate::input(input)?;
    let bucket = bucket(cx, put.bucket.as_deref())?;
    let key = checked_key(put.key.as_deref().unwrap_or(&put.workspace_file.name))?;
    let mut request = Request::new(Method::PUT, Some(bucket), key);
    let kind = put
        .content_type
        .as_deref()
        .unwrap_or("application/octet-stream");
    request.headers.push(("content-type", kind.into()));
    request.body = put.workspace_file.bytes()?;
    let size = request.body.len();
    let response = cx.send(signed(cx, request)?).await.map_err(tidy)?;
    let etag = response
        .headers()
        .get("etag")
        .and_then(|tag| tag.to_str().ok());
    Ok(json!({"bucket": bucket, "key": key, "size": size, "etag": etag}))
}

async fn delete(cx: &Context, input: Value) -> Result<Value, Error> {
    let object: Object = crate::input(input)?;
    let bucket = bucket(cx, object.bucket.as_deref())?;
    let key = checked_key(&object.key)?;
    let request = signed(cx, Request::new(Method::DELETE, Some(bucket), key))?;
    cx.send(request).await.map_err(tidy)?;
    Ok(json!({"deleted": key, "bucket": bucket}))
}

/// A copy can fail after its 200 answer has started, so the body is checked for an error too.
async fn copy(cx: &Context, input: Value) -> Result<Value, Error> {
    let copy: Copy = crate::input(input)?;
    let bucket = bucket(cx, copy.bucket.as_deref())?;
    let key = checked_key(&copy.key)?;
    let source_bucket = match copy.source_bucket.as_deref() {
        Some(named) => checked_bucket(named)?,
        None => bucket,
    };
    let source_key = checked_key(&copy.source_key)?;
    let mut request = Request::new(Method::PUT, Some(bucket), key);
    let source = format!("/{source_bucket}/{}", encode(source_key, true));
    request.headers.push(("x-amz-copy-source", source));
    let (body, _) = cx.bytes(signed(cx, request)?).await.map_err(tidy)?;
    let document = xml(&body)?;
    let root = document.root_element();
    if root.tag_name().name() == "Error" {
        let code = text(root, "Code").unwrap_or("Error");
        let message = text(root, "Message").unwrap_or("the copy failed");
        return Err(Error::Rejected(format!("{code}: {message}")));
    }
    Ok(json!({
        "bucket": bucket,
        "key": key,
        "source": format!("{source_bucket}/{source_key}"),
        "etag": text(root, "ETag"),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Credential,
        testing::{Exchange, Fixture},
    };

    const SECRET: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";

    fn credential() -> Credential {
        Credential::Fields {
            fields: [("secret_access_key".into(), SECRET.into())].into(),
        }
    }

    fn cx(config: Value, fixture: &Fixture) -> Context {
        let mut config = config;
        config["access_key_id"] = json!("AKIAIOSFODNN7EXAMPLE");
        Context::new(config, credential()).with_origin(&fixture.origin())
    }

    /// The examples in AWS's Signature Version 4 documentation for S3
    /// (https://docs.aws.amazon.com/AmazonS3/latest/API/sig-v4-header-based-auth.html) and the
    /// get-vanilla case of its published test suite.
    #[test]
    fn signatures_match_aws_published_examples() {
        let s3 = |date| Signing {
            key_id: "AKIAIOSFODNN7EXAMPLE",
            secret: SECRET,
            region: "us-east-1",
            service: "s3",
            date,
        };
        let host = ("host", "examplebucket.s3.amazonaws.com");
        let date = ("x-amz-date", "20130524T000000Z");
        let empty = ("x-amz-content-sha256", EMPTY);
        let get = Canonical {
            method: "GET",
            path: "/test.txt",
            query: "",
            headers: vec![host, ("range", "bytes=0-9"), empty, date],
            payload: EMPTY,
        };
        let signed = authorization(&s3("20130524T000000Z"), &get);
        assert_eq!(
            signed,
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, SignedHeaders=host;range;x-amz-content-sha256;x-amz-date, Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
        let put_hash = "44ce7dd67c959e0d3524ffac1771dfbba87d2b6b4b4e99e42034a8b803f8b072";
        assert_eq!(hex(&Sha256::digest(b"Welcome to Amazon S3.")), put_hash);
        let path = format!("/{}", encode("test$file.text", true));
        let put = Canonical {
            method: "PUT",
            path: &path,
            query: "",
            headers: vec![
                ("date", "Fri, 24 May 2013 00:00:00 GMT"),
                host,
                date,
                ("x-amz-storage-class", "REDUCED_REDUNDANCY"),
                ("x-amz-content-sha256", put_hash),
            ],
            payload: put_hash,
        };
        let signed = authorization(&s3("20130524T000000Z"), &put);
        assert!(
            signed.ends_with(
                "Signature=98ad721746da40c64f1a55b78f14c238d841ea1380cd77a1b5971af0ece108bd"
            ),
            "{signed}"
        );
        let listing = query(&[("prefix", "J".into()), ("max-keys", "2".into())]);
        assert_eq!(listing, "max-keys=2&prefix=J");
        let list = Canonical {
            method: "GET",
            path: "/",
            query: &listing,
            headers: vec![host, empty, date],
            payload: EMPTY,
        };
        let signed = authorization(&s3("20130524T000000Z"), &list);
        assert!(
            signed.ends_with(
                "Signature=34b48302e7b5fa45bde8084f4b7868a86f0a534bc59db6670ed5711ef69dc6f7"
            ),
            "{signed}"
        );
        let vanilla = Signing {
            key_id: "AKIDEXAMPLE",
            secret: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            region: "us-east-1",
            service: "service",
            date: "20150830T123600Z",
        };
        let request = Canonical {
            method: "GET",
            path: "/",
            query: "",
            headers: vec![
                ("host", "example.amazonaws.com"),
                ("x-amz-date", "20150830T123600Z"),
            ],
            payload: EMPTY,
        };
        assert_eq!(
            authorization(&vanilla, &request),
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, SignedHeaders=host;x-amz-date, Signature=5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"
        );
    }

    #[test]
    fn keys_and_buckets_that_would_reach_elsewhere_are_refused() {
        assert_eq!(encode("a b/ü+%.txt", true), "a%20b/%C3%BC%2B%25.txt");
        assert_eq!(encode("a/b", false), "a%2Fb");
        assert!(checked_key("reports/../secrets").is_err());
        assert!(checked_key("./a").is_err());
        assert!(checked_key("").is_err());
        assert!(checked_key("reports/2026/q1..csv").is_ok());
        assert!(checked_bucket("my-bucket").is_ok());
        assert!(checked_bucket("a/b").is_err());
        assert!(checked_bucket("ab").is_err());
    }

    const LIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Name>reports</Name><Prefix>2026/</Prefix><KeyCount>2</KeyCount><MaxKeys>2</MaxKeys><Delimiter>/</Delimiter><IsTruncated>true</IsTruncated><Contents><Key>2026/q1.csv</Key><LastModified>2026-11-06T10:00:00.000Z</LastModified><ETag>&#34;9b2cf535f27731c974343645a3985328&#34;</ETag><Size>1234</Size><StorageClass>STANDARD</StorageClass></Contents><CommonPrefixes><Prefix>2026/raw/</Prefix></CommonPrefixes><NextContinuationToken>1ueGcxLPRx1Tr/XYExHnhbYLgveDs2J/wm36Hy4vbOwM=</NextContinuationToken></ListBucketResult>"#;

    #[tokio::test]
    async fn objects_are_listed_read_and_inspected_on_a_path_style_endpoint() {
        let fixture = Fixture::start(vec![
            Exchange::raw("GET", "/reports", 200, LIST),
            Exchange::raw("GET", "/reports/2026/q1%20final.csv", 200, "a,b\n1,2\n")
                .header("content-type", "text/csv"),
            Exchange::raw("HEAD", "/reports/2026/q1.csv", 200, "")
                .header("etag", "\"9b2c\"")
                .header("x-amz-meta-owner", "finance"),
            Exchange::raw("HEAD", "/reports/missing", 404, ""),
        ])
        .await;
        let cx = cx(
            json!({"endpoint": fixture.origin(), "bucket": "reports"}),
            &fixture,
        );
        let input = json!({"prefix": "2026/", "delimiter": "/", "limit": 2});
        let listed = S3.call(&cx, "list_objects", input).await.unwrap();
        assert_eq!(
            listed["objects"][0],
            json!({"key": "2026/q1.csv", "size": 1234, "modified": "2026-11-06T10:00:00.000Z", "etag": "\"9b2cf535f27731c974343645a3985328\""})
        );
        assert_eq!(listed["folders"], json!(["2026/raw/"]));
        assert_eq!(
            listed["continuation"],
            "1ueGcxLPRx1Tr/XYExHnhbYLgveDs2J/wm36Hy4vbOwM="
        );
        let file = S3
            .call(&cx, "get_object", json!({"key": "2026/q1 final.csv"}))
            .await
            .unwrap();
        assert_eq!(file[crate::FILE]["name"], "q1 final.csv");
        assert_eq!(file[crate::FILE]["content_type"], "text/csv");
        let head = S3
            .call(&cx, "head_object", json!({"key": "2026/q1.csv"}))
            .await
            .unwrap();
        assert_eq!(head["metadata"], json!({"owner": "finance"}));
        let missing = S3.call(&cx, "head_object", json!({"key": "missing"})).await;
        assert_eq!(
            missing,
            Err(Error::NotFound("no object missing in reports".into()))
        );
        let seen = fixture.finish().await;
        assert_eq!(
            seen[0].target,
            "/reports?delimiter=%2F&list-type=2&max-keys=2&prefix=2026%2F"
        );
        let authorization = seen[0].header("authorization").unwrap();
        assert!(authorization.starts_with("AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/"));
        assert!(authorization.contains("/us-east-1/s3/aws4_request, SignedHeaders=host;x-amz-content-sha256;x-amz-date, Signature="));
        assert!(!authorization.contains(SECRET));
        assert_eq!(seen[0].header("x-amz-content-sha256"), Some(EMPTY));
    }

    #[tokio::test]
    async fn uploads_copies_and_deletes_are_signed_with_their_body_and_headers() {
        let fixture = Fixture::start(vec![
            Exchange::raw("PUT", "/reports.s3.eu-west-1.amazonaws.com/in/a.csv", 200, "")
                .header("etag", "\"abc\""),
            Exchange::raw(
                "PUT",
                "/reports.s3.eu-west-1.amazonaws.com/b.csv",
                200,
                "<CopyObjectResult><ETag>\"abc\"</ETag></CopyObjectResult>",
            ),
            Exchange::raw(
                "PUT",
                "/reports.s3.eu-west-1.amazonaws.com/c.csv",
                200,
                "<Error><Code>InternalError</Code><Message>We encountered an internal error.</Message></Error>",
            ),
            Exchange::raw("DELETE", "/reports.s3.eu-west-1.amazonaws.com/in/a.csv", 204, ""),
        ])
        .await;
        let cx = cx(
            json!({"region": "eu-west-1", "bucket": "reports"}),
            &fixture,
        );
        let file = Attachment::new("a.csv", b"a,b\n");
        let input = json!({"key": "in/a.csv", "workspace_file": file, "content_type": "text/csv"});
        let put = S3.call(&cx, "put_object", input).await.unwrap();
        assert_eq!(
            put,
            json!({"bucket": "reports", "key": "in/a.csv", "size": 4, "etag": "\"abc\""})
        );
        let input = json!({"key": "b.csv", "source_key": "in/a b.csv"});
        S3.call(&cx, "copy_object", input).await.unwrap();
        let failed = S3
            .call(
                &cx,
                "copy_object",
                json!({"key": "c.csv", "source_key": "in/a.csv"}),
            )
            .await;
        assert_eq!(
            failed,
            Err(Error::Rejected(
                "InternalError: We encountered an internal error.".into()
            ))
        );
        S3.call(&cx, "delete_object", json!({"key": "in/a.csv"}))
            .await
            .unwrap();
        let seen = fixture.finish().await;
        assert_eq!(seen[0].text(), "a,b\n");
        assert_eq!(
            seen[0].header("x-amz-content-sha256"),
            Some(hex(&Sha256::digest(b"a,b\n")).as_str())
        );
        assert!(seen[0].header("authorization").unwrap().contains("/eu-west-1/s3/aws4_request, SignedHeaders=content-type;host;x-amz-content-sha256;x-amz-date,"));
        assert_eq!(
            seen[1].header("x-amz-copy-source"),
            Some("/reports/in/a%20b.csv")
        );
        assert!(
            seen[1]
                .header("authorization")
                .unwrap()
                .contains("x-amz-copy-source;x-amz-date")
        );
    }

    #[tokio::test]
    async fn refusals_read_s3_error_codes_and_bad_input_never_reaches_the_server() {
        let denied = "<Error><Code>AccessDenied</Code><Message>Access Denied</Message></Error>";
        let bad_key = "<Error><Code>InvalidAccessKeyId</Code><Message>The AWS Access Key Id you provided does not exist in our records.</Message></Error>";
        let fixture = Fixture::start(vec![
            Exchange::raw("GET", "/", 403, bad_key),
            Exchange::raw("GET", "/reports/a.csv", 403, denied),
        ])
        .await;
        let cx = cx(json!({"endpoint": fixture.origin()}), &fixture);
        assert_eq!(
            S3.call(&cx, "list_buckets", json!({})).await,
            Err(Error::Unauthorized)
        );
        let refused = S3
            .call(
                &cx,
                "get_object",
                json!({"bucket": "reports", "key": "a.csv"}),
            )
            .await;
        assert_eq!(
            refused,
            Err(Error::Forbidden("AccessDenied: Access Denied".into()))
        );
        let bad = [
            ("list_objects", json!({})),
            ("get_object", json!({"bucket": "reports", "key": "a/../b"})),
            ("get_object", json!({"bucket": "../x", "key": "a"})),
            (
                "put_object",
                json!({"bucket": "reports", "workspace_file": "a.csv"}),
            ),
            ("list_buckets", json!({"bucket": "reports"})),
        ];
        for (tool, input) in bad {
            let refused = S3.call(&cx, tool, input).await;
            assert!(
                matches!(refused, Err(Error::Invalid(_))),
                "{tool}: {refused:?}"
            );
        }
        fixture.finish().await;
    }

    /// Every tool against a real server, such as MinIO: set `CUBEBOT_LIVE_S3_ENDPOINT` (empty for
    /// AWS), `CUBEBOT_LIVE_S3_REGION`, `CUBEBOT_LIVE_S3_BUCKET` (an existing bucket it may write
    /// in), `CUBEBOT_LIVE_S3_KEY_ID` and `CUBEBOT_LIVE_S3_SECRET`, then run
    /// `cargo test -p cubebot-connectors live_s3 -- --ignored`.
    #[tokio::test]
    #[ignore = "needs an S3-compatible server"]
    async fn live_s3() {
        let env = |key: &str| std::env::var(format!("CUBEBOT_LIVE_S3_{key}")).unwrap();
        let credential = Credential::Fields {
            fields: [("secret_access_key".into(), env("SECRET"))].into(),
        };
        let config = json!({"endpoint": env("ENDPOINT"), "region": env("REGION"), "bucket": env("BUCKET"), "access_key_id": env("KEY_ID")});
        let cx = Context::new(config, credential);
        println!("check: {:?}", S3.check(&cx).await.unwrap());
        let call = |tool: &'static str, input: Value| {
            let cx = &cx;
            async move {
                let answer = S3.call(cx, tool, input).await;
                let shown = format!("{answer:?}");
                println!("{tool}: {}", shown.chars().take(500).collect::<String>());
                answer
            }
        };
        let listed = call("list_buckets", json!({})).await.unwrap();
        assert!(listed["buckets"].to_string().contains(&env("BUCKET")));
        let stamp = crate::dav::now_nanos();
        let key = format!("cubebot-live/{stamp}/report ü.csv");
        let copied = format!("cubebot-live/{stamp}/copy.csv");
        let file = Attachment::new("report.csv", b"a,b\n1,2\n");
        let input = json!({"key": key, "workspace_file": file, "content_type": "text/csv"});
        call("put_object", input).await.unwrap();
        let head = call("head_object", json!({"key": key})).await.unwrap();
        assert_eq!(
            (head["size"].clone(), head["content_type"].clone()),
            (json!(8), json!("text/csv"))
        );
        let got = call("get_object", json!({"key": key})).await.unwrap();
        assert_eq!(got[crate::FILE]["base64"], "YSxiCjEsMgo=");
        call("copy_object", json!({"key": copied, "source_key": key}))
            .await
            .unwrap();
        let prefix = format!("cubebot-live/{stamp}/");
        let listed = call("list_objects", json!({"prefix": prefix, "limit": 1}))
            .await
            .unwrap();
        assert_eq!(listed["objects"].as_array().unwrap().len(), 1);
        let next = json!({"prefix": prefix, "continuation": listed["continuation"]});
        let rest = call("list_objects", next).await.unwrap();
        assert_eq!(rest["objects"].as_array().unwrap().len(), 1);
        let folders = call(
            "list_objects",
            json!({"prefix": "cubebot-live/", "delimiter": "/"}),
        )
        .await
        .unwrap();
        assert!(folders["folders"].to_string().contains(&prefix));
        for key in [&key, &copied] {
            call("delete_object", json!({"key": key})).await.unwrap();
        }
        let gone = call("head_object", json!({"key": key})).await;
        assert!(matches!(gone, Err(Error::NotFound(_))));
        let wrong = Credential::Fields {
            fields: [("secret_access_key".into(), "not-the-secret".into())].into(),
        };
        let config = json!({"endpoint": env("ENDPOINT"), "region": env("REGION"), "access_key_id": env("KEY_ID")});
        let refused = S3.check(&Context::new(config, wrong)).await;
        assert_eq!(refused, Err(Error::Unauthorized));
    }
}
