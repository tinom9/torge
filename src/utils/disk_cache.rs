use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::PathBuf};
use thiserror::Error;

/// Marker value stored in cache to indicate a selector was not found in Sourcify.
pub const CACHE_MISS_MARKER: &str = "<UNKNOWN>";

#[derive(Debug, Error)]
pub enum CacheError {
    #[error("could not determine cache directory")]
    NoCacheDir,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Disk cache for selector lookups.
#[derive(Serialize, Deserialize, Default)]
pub struct DiskCache {
    selectors: HashMap<String, String>,
}

impl DiskCache {
    /// Get the path to the cache file.
    ///
    /// Respects `XDG_CACHE_HOME` on all platforms (not just Linux) so that
    /// integration tests using a temp directory work on macOS/Windows too.
    pub fn cache_path() -> Option<PathBuf> {
        let base = std::env::var("XDG_CACHE_HOME")
            .ok()
            .map(PathBuf::from)
            .or_else(dirs::cache_dir)?;
        Some(base.join("torge").join("selectors.json"))
    }

    fn is_disabled() -> bool {
        std::env::var("TORGE_DISABLE_CACHE").is_ok()
    }

    pub fn load() -> Self {
        if Self::is_disabled() {
            return Self::default();
        }

        let Some(path) = Self::cache_path() else {
            return Self::default();
        };

        match fs::read_to_string(&path) {
            Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) {
        if Self::is_disabled() {
            return;
        }

        let Some(path) = Self::cache_path() else {
            return;
        };

        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        let contents = match serde_json::to_string_pretty(self) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("warning: failed to serialize selector cache: {e}");
                return;
            }
        };

        // Atomic write: write to temp file then rename into place.
        let tmp_path = path.with_extension("json.tmp");
        if let Err(e) = fs::write(&tmp_path, contents) {
            eprintln!("warning: failed to write selector cache: {e}");
            return;
        }
        if let Err(e) = fs::rename(&tmp_path, &path) {
            eprintln!("warning: failed to rename selector cache: {e}");
        }
    }

    pub fn get(&self, selector: &str) -> Option<&str> {
        self.selectors.get(selector).map(String::as_str)
    }

    pub fn insert(&mut self, selector: String, signature: String) {
        self.selectors.insert(selector, signature);
    }

    /// Remove all unknown (unresolved) selectors from the persisted cache.
    /// Returns `(kept, removed)` counts.
    pub fn remove_unknown() -> Result<(usize, usize), CacheError> {
        let path = Self::cache_path().ok_or(CacheError::NoCacheDir)?;
        if !path.exists() {
            return Ok((0, 0));
        }
        let contents = fs::read_to_string(&path)?;
        let mut cache: DiskCache = serde_json::from_str(&contents)?;

        let before = cache.selectors.len();
        cache.selectors.retain(|_, v| v != CACHE_MISS_MARKER);
        let after = cache.selectors.len();

        let json = serde_json::to_string_pretty(&cache)?;
        let tmp_path = path.with_extension("json.tmp");
        fs::write(&tmp_path, json)?;
        fs::rename(&tmp_path, &path)?;

        Ok((after, before - after))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_cache_is_empty() {
        let cache = DiskCache::default();
        assert!(cache.get("0x12345678").is_none());
    }

    #[test]
    fn test_insert_and_get() {
        let mut cache = DiskCache::default();
        cache.insert(
            "0x12345678".to_owned(),
            "transfer(address,uint256)".to_owned(),
        );

        assert_eq!(cache.get("0x12345678"), Some("transfer(address,uint256)"));
        assert!(cache.get("0xdeadbeef").is_none());
    }

    #[test]
    fn test_cache_miss_marker_persistence() {
        let mut cache = DiskCache::default();
        cache.insert("0xdeadbeef".to_owned(), CACHE_MISS_MARKER.to_owned());

        assert_eq!(cache.get("0xdeadbeef"), Some(CACHE_MISS_MARKER));
    }

    #[test]
    fn test_overwrite_existing_entry() {
        let mut cache = DiskCache::default();
        cache.insert("0x12345678".to_owned(), "oldSignature()".to_owned());
        cache.insert("0x12345678".to_owned(), "newSignature(uint256)".to_owned());

        assert_eq!(cache.get("0x12345678"), Some("newSignature(uint256)"));
    }
}
