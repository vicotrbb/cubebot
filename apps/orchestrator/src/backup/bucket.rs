//! The backup bucket, reached with the S3 connector's Signature Version 4 signer on Cubebot's own
//! HTTP client: an operator's bucket is often on a private address, which agents' connectors refuse.
use super::Failure;
use crate::identity::Source;
use axum::body::Bytes;
use cubebot_connectors::s3::{self, Signer};
use futures_util::{Stream, StreamExt};
use reqwest::Method;
use sha2::{Digest, Sha256};
use std::{sync::Mutex, time::Duration};
use tokio::time::Instant;

/// S3 needs at least 5 MiB in every multipart part but the last; 10,000 parts make 160 GiB.
const PART: usize = 16 << 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    /// AWS S3 in `region` when absent.
    pub endpoint: Option<String>,
    pub region: String,
    pub bucket: String,
    pub access_key_id: String,
    pub secret_access_key: String,
}

#[derive(Clone)]
pub struct Bucket {
    http: reqwest::Client,
    target: Target,
}

/// One byte rate shared by every transfer of a backup.
pub struct Pace {
    rate: f64,
    next: Mutex<Instant>,
}

impl Pace {
    pub fn new(bytes_per_second: u64) -> Self {
        Self {
            rate: bytes_per_second.max(1) as f64,
            next: Mutex::new(Instant::now()),
        }
    }

    /// Waits for this many bytes' turn.
    pub async fn take(&self, bytes: usize) {
        let start = {
            let mut next = self
                .next
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let start = (*next).max(Instant::now());
            *next = start + Duration::from_secs_f64(bytes as f64 / self.rate);
            start
        };
        tokio::time::sleep_until(start).await;
    }
}

/// A multipart upload in progress: its id and the ETag of each part sent.
struct Multipart {
    id: String,
    tags: Vec<String>,
}

impl Bucket {
    pub fn new(target: Target) -> Result<Self, Failure> {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(120))
            .build()
            .map_err(|error| Failure::caused("the bucket client could not start", error))?;
        Ok(Self { http, target })
    }

    pub fn name(&self) -> &str {
        &self.target.bucket
    }

    async fn send(
        &self,
        method: Method,
        key: &str,
        query: &[(&'static str, String)],
        body: Bytes,
    ) -> Result<reqwest::Response, Failure> {
        let signer = Signer {
            endpoint: self.target.endpoint.as_deref(),
            addressing: None,
            region: &self.target.region,
            key_id: &self.target.access_key_id,
            secret: &self.target.secret_access_key,
        };
        let payload = format!("{:x}", Sha256::digest(&body));
        let what = format!("{method} {key}");
        let signed = signer
            .sign(
                &method,
                Some(&self.target.bucket),
                key,
                query,
                Vec::new(),
                &payload,
            )
            .map_err(|error| Failure::new(format!("the bucket settings are invalid: {error}")))?;
        let mut request = self.http.request(method, &signed.url).body(body);
        for (name, value) in signed.headers {
            request = request.header(name, value);
        }
        let response = request.send().await.map_err(|error| {
            Failure::caused(
                format!("the bucket could not be reached for {what}"),
                error.without_url(),
            )
        })?;
        if response.status().is_success() {
            return Ok(response);
        }
        let status = response.status().as_u16();
        let text = response.bytes().await.unwrap_or_default();
        let reason = match s3::answered(&text, "Code") {
            Err(refused) => format!(" ({refused})"),
            Ok(_) => String::new(),
        };
        Err(Failure::new(format!(
            "the bucket answered {what} with {status}{reason}"
        )))
    }

    async fn read(&self, response: reqwest::Response, key: &str) -> Result<Bytes, Failure> {
        response.bytes().await.map_err(|error| {
            Failure::caused(
                format!("the bucket's answer for {key} was cut short"),
                error.without_url(),
            )
        })
    }

    pub async fn put(&self, key: &str, body: Vec<u8>) -> Result<(), Failure> {
        self.send(Method::PUT, key, &[], body.into())
            .await
            .map(drop)
    }

    /// The object's bytes as they arrive.
    pub async fn get(&self, key: &str) -> Result<reqwest::Response, Failure> {
        self.send(Method::GET, key, &[], Bytes::new()).await
    }

    pub async fn bytes(&self, key: &str) -> Result<Bytes, Failure> {
        let response = self.get(key).await?;
        self.read(response, key).await
    }

    pub async fn delete(&self, key: &str) -> Result<(), Failure> {
        self.send(Method::DELETE, key, &[], Bytes::new())
            .await
            .map(drop)
    }

    /// Every key under `prefix`, with its size.
    pub async fn list(&self, prefix: &str) -> Result<Vec<(String, u64)>, Failure> {
        let mut all = Vec::new();
        let mut token = None;
        loop {
            let mut query = vec![("list-type", "2".to_owned()), ("prefix", prefix.to_owned())];
            query.extend(token.take().map(|token| ("continuation-token", token)));
            let response = self.send(Method::GET, "", &query, Bytes::new()).await?;
            let body = self.read(response, prefix).await?;
            let page = s3::listed(&body).map_err(|error| {
                Failure::new(format!("the bucket's listing was unreadable: {error}"))
            })?;
            all.extend(page.objects);
            match page.next {
                Some(next) => token = Some(next),
                None => return Ok(all),
            }
        }
    }

    /// Streams `body` to `key` at `pace`, returning its size and SHA-256. A body of one part is
    /// one PUT; a longer one a multipart upload, aborted if anything fails.
    pub async fn upload<S, E>(
        &self,
        key: &str,
        mut body: S,
        pace: &Pace,
    ) -> Result<(u64, [u8; 32]), Failure>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin,
        E: Into<Source>,
    {
        let (mut hash, mut size) = (Sha256::new(), 0);
        let mut part = Vec::with_capacity(PART);
        let mut multipart = None;
        let sent = async {
            while let Some(chunk) = body.next().await {
                let chunk =
                    chunk.map_err(|error| Failure::caused(format!("{key} ended early"), error))?;
                hash.update(&chunk);
                size += chunk.len() as u64;
                part.extend_from_slice(&chunk);
                if part.len() >= PART {
                    self.part(key, &mut multipart, std::mem::take(&mut part), pace)
                        .await?;
                }
            }
            if multipart.is_none() {
                pace.take(part.len()).await;
                return self.put(key, part).await;
            }
            if !part.is_empty() {
                self.part(key, &mut multipart, part, pace).await?;
            }
            self.complete(key, multipart.as_ref()).await
        }
        .await;
        if let (Err(_), Some(upload)) = (&sent, &multipart) {
            let query = [("uploadId", upload.id.clone())];
            let _ = self.send(Method::DELETE, key, &query, Bytes::new()).await;
        }
        sent.map(|()| (size, hash.finalize().into()))
    }

    async fn part(
        &self,
        key: &str,
        multipart: &mut Option<Multipart>,
        bytes: Vec<u8>,
        pace: &Pace,
    ) -> Result<(), Failure> {
        if multipart.is_none() {
            let response = self
                .send(
                    Method::POST,
                    key,
                    &[("uploads", String::new())],
                    Bytes::new(),
                )
                .await?;
            let body = self.read(response, key).await?;
            let id = s3::answered(&body, "UploadId").ok().flatten();
            let id = id.ok_or_else(|| {
                Failure::new(format!(
                    "the bucket did not start a multipart upload of {key}"
                ))
            })?;
            *multipart = Some(Multipart {
                id,
                tags: Vec::new(),
            });
        }
        let Some(upload) = multipart.as_mut() else {
            return Ok(());
        };
        pace.take(bytes.len()).await;
        let query = [
            ("partNumber", (upload.tags.len() + 1).to_string()),
            ("uploadId", upload.id.clone()),
        ];
        let response = self.send(Method::PUT, key, &query, bytes.into()).await?;
        let tag = response
            .headers()
            .get("etag")
            .and_then(|tag| tag.to_str().ok());
        let tag = tag
            .ok_or_else(|| Failure::new(format!("the bucket gave no ETag for a part of {key}")))?;
        upload.tags.push(tag.to_owned());
        Ok(())
    }

    async fn complete(&self, key: &str, multipart: Option<&Multipart>) -> Result<(), Failure> {
        let Some(upload) = multipart else {
            return Ok(());
        };
        let parts: String = upload
            .tags
            .iter()
            .enumerate()
            .map(|(index, tag)| {
                format!(
                    "<Part><PartNumber>{}</PartNumber><ETag>{tag}</ETag></Part>",
                    index + 1
                )
            })
            .collect();
        let body = format!("<CompleteMultipartUpload>{parts}</CompleteMultipartUpload>");
        let response = self
            .send(
                Method::POST,
                key,
                &[("uploadId", upload.id.clone())],
                body.into(),
            )
            .await?;
        let answer = self.read(response, key).await?;
        s3::answered(&answer, "ETag")
            .map(drop)
            .map_err(|error| Failure::new(format!("the bucket did not complete {key}: {error}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_pace_spreads_transfers_at_its_rate() {
        let pace = Pace::new(100_000);
        let start = Instant::now();
        for _ in 0..4 {
            pace.take(10_000).await;
        }
        // The first goes at once and each later one a tenth of a second after the one before.
        let elapsed = start.elapsed();
        assert!(elapsed >= Duration::from_millis(300), "{elapsed:?}");
        assert!(elapsed < Duration::from_millis(400), "{elapsed:?}");
    }
}
