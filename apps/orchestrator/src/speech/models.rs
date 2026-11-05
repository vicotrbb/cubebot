use super::SpeechError;
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

const RELEASES: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download";

pub struct Asset {
    pub name: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub file: &'static str,
    pub keep: &'static [&'static str],
}

pub const SILERO: Asset = Asset {
    name: "silero_vad",
    url: "asr-models/silero_vad.onnx",
    sha256: "9e2449e1087496d8d4caba907f23e0bd3f78d91fa552479bb9c23ac09cbb1fd6",
    file: "silero_vad.onnx",
    keep: &[],
};
pub const ZIPFORMER: Asset = Asset {
    name: "sherpa-onnx-streaming-zipformer-en-2023-06-26",
    url: "asr-models/sherpa-onnx-streaming-zipformer-en-2023-06-26.tar.bz2",
    sha256: "639e25b578e9e997131402199419c13a941f8e4e198e2da1ce57dbf5cf401282",
    file: "",
    keep: &[".int8.onnx", "tokens.txt", "/test_wavs/"],
};
pub const LESSAC: Asset = Asset {
    name: "vits-piper-en_US-lessac-medium",
    url: "tts-models/vits-piper-en_US-lessac-medium.tar.bz2",
    sha256: "9e3febfacf0abf4270172d2958bcec246032b7e88efc2720840cc80c93de334e",
    file: "en_US-lessac-medium.onnx",
    keep: &[".onnx", "tokens.txt", "/espeak-ng-data/"],
};
pub const RYAN: Asset = Asset {
    name: "vits-piper-en_US-ryan-medium",
    url: "tts-models/vits-piper-en_US-ryan-medium.tar.bz2",
    sha256: "c546af78b6395b4e7c4ce1ed899438b64426a362f5d4ec5fecd090ded9ad7505",
    file: "en_US-ryan-medium.onnx",
    keep: &[".onnx", "tokens.txt", "/espeak-ng-data/"],
};

pub struct Models {
    root: PathBuf,
    releases: String,
    gate: tokio::sync::Mutex<()>,
}

impl Models {
    pub fn new(root: PathBuf) -> Self {
        Self::from(root, RELEASES.to_owned())
    }

    pub fn from(root: PathBuf, releases: String) -> Self {
        Self {
            root,
            releases,
            gate: tokio::sync::Mutex::new(()),
        }
    }

    #[tracing::instrument(skip_all, fields(asset = asset.name))]
    pub async fn ensure(
        &self,
        asset: &Asset,
        progress: &(dyn Fn(&'static str, u64, u64) + Sync),
    ) -> Result<PathBuf, SpeechError> {
        let path = self.root.join(asset.name);
        if path.is_dir() {
            return Ok(path);
        }
        let _gate = self.gate.lock().await;
        if path.is_dir() {
            return Ok(path);
        }
        tokio::fs::create_dir_all(&self.root).await?;
        let staging = self.root.join(format!(".{}.staging", asset.name));
        let _ = tokio::fs::remove_dir_all(&staging).await;
        tokio::fs::create_dir_all(&staging).await?;
        let result = self.fetch(asset, &staging, &path, progress).await;
        let _ = tokio::fs::remove_dir_all(&staging).await;
        result?;
        tracing::info!(asset = asset.name, "speech model installed");
        Ok(path)
    }

    async fn fetch(
        &self,
        asset: &Asset,
        staging: &Path,
        path: &Path,
        progress: &(dyn Fn(&'static str, u64, u64) + Sync),
    ) -> Result<(), SpeechError> {
        let part = staging.join("download");
        self.download(asset, &part, progress).await?;
        let extracted = staging.join("extracted");
        let (from, keep) = (part.clone(), asset.keep);
        let file = asset.file;
        let out = extracted.clone();
        tokio::task::spawn_blocking(move || unpack(&from, &out, file, keep))
            .await
            .map_err(std::io::Error::other)??;
        let source = if asset.keep.is_empty() {
            extracted
        } else {
            extracted.join(asset.name)
        };
        tokio::fs::rename(source, path).await?;
        Ok(())
    }

    async fn download(
        &self,
        asset: &Asset,
        part: &Path,
        progress: &(dyn Fn(&'static str, u64, u64) + Sync),
    ) -> Result<(), SpeechError> {
        let url = format!("{}/{}", self.releases, asset.url);
        let response = reqwest::get(url).await?;
        if !response.status().is_success() {
            return Err(SpeechError::Refused(response.status().as_u16()));
        }
        let total = response.content_length().unwrap_or(0);
        let mut body = response.bytes_stream();
        let mut file = tokio::fs::File::create(part).await?;
        let mut hash = Sha256::new();
        let (mut received, mut reported) = (0u64, 0u64);
        while let Some(chunk) = body.next().await {
            let chunk = chunk?;
            hash.update(&chunk);
            file.write_all(&chunk).await?;
            received += chunk.len() as u64;
            if received - reported >= total / 50 {
                progress(asset.name, received, total);
                reported = received;
            }
        }
        file.flush().await?;
        if hex(&hash.finalize()) != asset.sha256 {
            return Err(SpeechError::Checksum(asset.name));
        }
        Ok(())
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unpack(from: &Path, to: &Path, file: &str, keep: &[&str]) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    if keep.is_empty() {
        return std::fs::rename(from, to.join(file));
    }
    let decoded = bzip2::read::BzDecoder::new(std::fs::File::open(from)?);
    let mut archive = tar::Archive::new(decoded);
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_string_lossy().into_owned();
        if keep
            .iter()
            .any(|kept| path.ends_with(kept) || (kept.ends_with('/') && path.contains(kept)))
        {
            // Refuses a member that would land outside `to`.
            entry.unpack_in(to)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, routing::get};

    fn archive() -> Vec<u8> {
        let mut tar = tar::Builder::new(bzip2::write::BzEncoder::new(
            Vec::new(),
            bzip2::Compression::fast(),
        ));
        for (path, body) in [
            ("m/a.int8.onnx", &b"kept"[..]),
            ("m/a.onnx", &b"dropped"[..]),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, path, body).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap()
    }

    async fn server(body: Vec<u8>) -> String {
        let app = Router::new().route("/m.tar.bz2", get(move || async move { body }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{address}")
    }

    fn leak(text: String) -> &'static str {
        Box::leak(text.into_boxed_str())
    }

    fn scratch() -> PathBuf {
        let root = std::env::temp_dir().join(format!("cubebot-models-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[tokio::test]
    async fn a_good_download_extracts_only_kept_members_and_is_not_fetched_twice() {
        let body = archive();
        let asset = Asset {
            name: "m",
            url: "m.tar.bz2",
            sha256: leak(hex(&Sha256::digest(&body))),
            file: "",
            keep: &[".int8.onnx"],
        };
        let root = scratch();
        let models = Models::from(root.clone(), server(body).await);
        let path = models.ensure(&asset, &|_, _, _| {}).await.unwrap();
        assert_eq!(std::fs::read(path.join("a.int8.onnx")).unwrap(), b"kept");
        assert!(!path.join("a.onnx").exists());
        let offline = Models::from(root.clone(), "http://127.0.0.1:9".to_owned());
        offline.ensure(&asset, &|_, _, _| {}).await.unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn a_tampered_download_is_refused_and_leaves_nothing_behind() {
        let asset = Asset {
            name: "m",
            url: "m.tar.bz2",
            sha256: "0000",
            file: "",
            keep: &[".int8.onnx"],
        };
        let root = scratch();
        let models = Models::from(root.clone(), server(archive()).await);
        let refused = models.ensure(&asset, &|_, _, _| {}).await;
        assert!(matches!(refused, Err(SpeechError::Checksum("m"))));
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        std::fs::remove_dir_all(root).unwrap();
    }
}
