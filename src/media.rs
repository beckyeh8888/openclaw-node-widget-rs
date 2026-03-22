use std::fs;
use std::io::{Error, ErrorKind, Result};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use chrono::Utc;
use uuid::Uuid;

use crate::config;

const MAX_FILE_BYTES: usize = 10 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct MediaStore {
    pub base_dir: PathBuf,
}

impl MediaStore {
    pub fn new() -> Self {
        let base = config::app_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("media");
        let _ = fs::create_dir_all(&base);
        Self { base_dir: base }
    }

    #[cfg(test)]
    pub fn new_for_tests(base_dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(&base_dir);
        Self { base_dir }
    }

    pub fn store_file(&self, data: &[u8], mime: &str) -> Result<String> {
        if data.len() > MAX_FILE_BYTES {
            return Err(Error::new(ErrorKind::InvalidData, "File too large"));
        }

        let month_dir = Utc::now().format("%Y-%m").to_string();
        let ext = extension_for_mime(mime);
        let file_name = format!("{}.{}", Uuid::now_v7(), ext);
        let rel = format!("{month_dir}/{file_name}");
        let full_dir = self.base_dir.join(&month_dir);
        fs::create_dir_all(&full_dir)?;
        fs::write(full_dir.join(file_name), data)?;
        Ok(rel)
    }

    pub fn store_from_url(&self, url: &str) -> Result<String> {
        let resp = reqwest::blocking::get(url)
            .map_err(|e| Error::new(ErrorKind::Other, format!("download failed: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(Error::new(
                ErrorKind::Other,
                format!("download status not successful: {status}"),
            ));
        }

        let mime = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.split(';').next().unwrap_or(s).trim().to_string())
            .unwrap_or_else(|| "application/octet-stream".to_string());

        let bytes = resp
            .bytes()
            .map_err(|e| Error::new(ErrorKind::Other, format!("read body failed: {e}")))?;
        self.store_file(&bytes, &mime)
    }

    pub fn get_full_path(&self, relative: &str) -> PathBuf {
        self.base_dir.join(relative)
    }

    pub fn cleanup_older_than_days(&self, days: u32) -> Result<u64> {
        let cutoff = SystemTime::now()
            .checked_sub(Duration::from_secs(u64::from(days) * 24 * 60 * 60))
            .unwrap_or(SystemTime::UNIX_EPOCH);

        let mut deleted = 0u64;
        self.walk_files(|path| {
            let metadata = fs::metadata(path)?;
            let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            if modified < cutoff {
                fs::remove_file(path)?;
                deleted += 1;
            }
            Ok(())
        })?;
        Ok(deleted)
    }

    pub fn total_size_bytes(&self) -> Result<u64> {
        let mut total = 0u64;
        self.walk_files(|path| {
            total = total.saturating_add(fs::metadata(path)?.len());
            Ok(())
        })?;
        Ok(total)
    }

    pub fn enforce_max_size_bytes(&self, max_bytes: u64) -> Result<u64> {
        let mut files: Vec<(PathBuf, SystemTime, u64)> = Vec::new();
        self.walk_files(|path| {
            let metadata = fs::metadata(path)?;
            files.push((
                path.to_path_buf(),
                metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                metadata.len(),
            ));
            Ok(())
        })?;

        files.sort_by_key(|(_, modified, _)| *modified);
        let mut total: u64 = files.iter().map(|(_, _, len)| *len).sum();
        let mut deleted = 0u64;

        for (path, _, len) in files {
            if total <= max_bytes {
                break;
            }
            if fs::remove_file(&path).is_ok() {
                total = total.saturating_sub(len);
                deleted += 1;
            }
        }

        Ok(deleted)
    }

    fn walk_files<F>(&self, mut f: F) -> Result<()>
    where
        F: FnMut(&Path) -> Result<()>,
    {
        if !self.base_dir.exists() {
            return Ok(());
        }

        let mut stack = vec![self.base_dir.clone()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.is_file() {
                    f(&path)?;
                }
            }
        }
        Ok(())
    }
}

fn extension_for_mime(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "application/pdf" => "pdf",
        "text/plain" => "txt",
        _ => "bin",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_media_store(name: &str) -> MediaStore {
        let dir = std::env::temp_dir().join("openclaw_test_media").join(name);
        let _ = fs::remove_dir_all(&dir);
        MediaStore::new_for_tests(dir)
    }

    #[test]
    fn test_media_store_and_retrieve() {
        let store = temp_media_store("store_and_retrieve");
        let rel = store
            .store_file(b"hello", "image/png")
            .expect("store should succeed");
        let full = store.get_full_path(&rel);
        assert!(full.exists());
        assert!(rel.ends_with(".png"));
    }

    #[test]
    fn test_media_cleanup_by_age() {
        let store = temp_media_store("cleanup_age");
        let rel = store.store_file(b"abc", "text/plain").unwrap();
        let full = store.get_full_path(&rel);

        let deleted = store.cleanup_older_than_days(0).unwrap();
        assert_eq!(deleted, 1);
        assert!(!full.exists());
    }

    #[test]
    fn test_media_cleanup_by_size() {
        let store = temp_media_store("cleanup_size");
        let one_mb = vec![1u8; 1024 * 1024];
        for _ in 0..3 {
            let _ = store.store_file(&one_mb, "application/octet-stream").unwrap();
        }

        let deleted = store.enforce_max_size_bytes(1024 * 1024).unwrap();
        assert!(deleted >= 2);
        let total = store.total_size_bytes().unwrap();
        assert!(total <= 1024 * 1024);
    }
}
