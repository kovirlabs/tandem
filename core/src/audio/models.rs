//! Model files (whisper.cpp ggml models, Piper voices) on disk under `<data_dir>/models/`.
//!
//! Downloads are **only** started by an explicit user action in Settings ("Download"), which
//! names the host first (CLAUDE.md rule 10: no network calls the user didn't ask for).
//! Model files are not user data; audio never goes here.

use std::path::{Path, PathBuf};

use futures::StreamExt;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use super::{AudioError, DownloadProgress};

/// A file to fetch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelFile {
    /// Path relative to the models dir, e.g. `whisper/ggml-base.en.bin`.
    pub rel_path: String,
    pub url: String,
    /// Lowercase hex SHA-256, if known. Verified after download.
    pub sha256: Option<String>,
}

pub fn path_of(models_dir: &Path, file: &ModelFile) -> PathBuf {
    models_dir.join(&file.rel_path)
}

pub fn is_present(models_dir: &Path, file: &ModelFile) -> bool {
    path_of(models_dir, file).is_file()
}

/// Download `file` into `models_dir` unless it is already there. Writes to `<name>.part` and
/// renames on success so a half-finished download never looks complete. `progress` is called
/// with `(done_bytes, total_bytes)`; `model` is the id used in `DownloadProgress`.
pub async fn ensure(
    models_dir: &Path,
    file: &ModelFile,
    model: &str,
    progress: &(dyn Fn(DownloadProgress) + Send + Sync),
) -> Result<PathBuf, AudioError> {
    let dest = path_of(models_dir, file);
    if dest.is_file() {
        return Ok(dest);
    }
    let report = |done: u64, total: Option<u64>, finished: bool, error: Option<String>| {
        progress(DownloadProgress {
            model: model.to_string(),
            done_bytes: done,
            total_bytes: total,
            finished,
            error,
        })
    };
    let result = download(&dest, file, &|d, t| report(d, t, false, None)).await;
    match &result {
        Ok(_) => report(0, None, true, None),
        Err(e) => report(0, None, true, Some(e.to_string())),
    }
    result.map(|_| dest)
}

async fn download(
    dest: &Path,
    file: &ModelFile,
    progress: &(dyn Fn(u64, Option<u64>) + Send + Sync),
) -> Result<(), AudioError> {
    let err = |e: &dyn std::fmt::Display| AudioError::Download(e.to_string());
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| err(&e))?;
    }
    let part = dest.with_extension(format!(
        "{}.part",
        dest.extension().and_then(|e| e.to_str()).unwrap_or("bin")
    ));
    let resp = reqwest::get(&file.url).await.map_err(|e| err(&e))?;
    let resp = resp.error_for_status().map_err(|e| err(&e))?;
    let total = resp.content_length();
    let mut out = tokio::fs::File::create(&part).await.map_err(|e| err(&e))?;
    let mut hasher = Sha256::new();
    let mut done: u64 = 0;
    let mut last_report: u64 = 0;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| err(&e))?;
        hasher.update(&chunk);
        out.write_all(&chunk).await.map_err(|e| err(&e))?;
        done += chunk.len() as u64;
        if done - last_report >= 512 * 1024 {
            last_report = done;
            progress(done, total);
        }
    }
    out.flush().await.map_err(|e| err(&e))?;
    drop(out);
    if let Some(expected) = &file.sha256 {
        let got = hex(&hasher.finalize());
        if !got.eq_ignore_ascii_case(expected) {
            let _ = tokio::fs::remove_file(&part).await;
            return Err(AudioError::Download(format!(
                "checksum mismatch for {}",
                file.rel_path
            )));
        }
    }
    tokio::fs::rename(&part, dest).await.map_err(|e| err(&e))?;
    progress(done, total);
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_encodes() {
        assert_eq!(hex(&[0x00, 0xab, 0xff]), "00abff");
    }

    #[tokio::test]
    async fn present_file_is_not_downloaded() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = ModelFile {
            rel_path: "whisper/x.bin".into(),
            url: "http://127.0.0.1:9/never".into(),
            sha256: None,
        };
        std::fs::create_dir_all(dir.path().join("whisper")).expect("mkdir");
        std::fs::write(path_of(dir.path(), &file), b"x").expect("write");
        let got = ensure(dir.path(), &file, "t", &|_| {}).await;
        assert!(got.is_ok());
    }

    #[tokio::test]
    async fn failed_download_reports_and_leaves_no_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = ModelFile {
            rel_path: "whisper/y.bin".into(),
            url: "http://127.0.0.1:9/never".into(),
            sha256: None,
        };
        let seen = std::sync::Mutex::new(Vec::new());
        let got = ensure(dir.path(), &file, "t", &|p| {
            if let Ok(mut g) = seen.lock() {
                g.push(p);
            }
        })
        .await;
        assert!(got.is_err());
        assert!(!is_present(dir.path(), &file));
        let seen = seen.lock().map(|g| g.clone()).unwrap_or_default();
        assert!(seen.last().is_some_and(|p| p.finished && p.error.is_some()));
    }
}
