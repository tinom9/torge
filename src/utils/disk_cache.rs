use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::PathBuf};

/// Marker value stored in cache to indicate a selector was not found in Sourcify.
pub const CACHE_MISS_MARKER: &str = "<UNKNOWN>";

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
        // If cache is disabled (e.g., in tests), return empty cache.
        if Self::is_disabled() {
            return Self::default();
        }

        let path = match Self::cache_path() {
            Some(p) => p,
            None => return Self::default(),
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

        let path = match Self::cache_path() {
            Some(p) => p,
            None => return,
        };

        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        let contents = match serde_json::to_string_pretty(self) {
            Ok(s) => s,
            Err(_) => return,
        };

        let _ = fs::write(path, contents);
    }

    pub fn get(&self, selector: &str) -> Option<&String> {
        self.selectors.get(selector)
    }

    pub fn insert(&mut self, selector: String, signature: String) {
        self.selectors.insert(selector, signature);
    }

    /// Remove all unknown (unresolved) selectors from the persisted cache.
    /// Returns `(kept, removed)` counts.
    pub fn remove_unknown() -> Result<(usize, usize), String> {
        let path = Self::cache_path().ok_or("could not determine cache directory")?;
        if !path.exists() {
            return Ok((0, 0));
        }
        let contents =
            fs::read_to_string(&path).map_err(|e| format!("failed to read cache: {e}"))?;
        let mut cache: DiskCache =
            serde_json::from_str(&contents).map_err(|e| format!("failed to parse cache: {e}"))?;

        let before = cache.selectors.len();
        cache.selectors.retain(|_, v| v != CACHE_MISS_MARKER);
        let after = cache.selectors.len();

        let json = serde_json::to_string_pretty(&cache)
            .map_err(|e| format!("failed to serialize cache: {e}"))?;
        fs::write(&path, json).map_err(|e| format!("failed to write cache: {e}"))?;

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

        assert_eq!(
            cache.get("0x12345678"),
            Some(&"transfer(address,uint256)".to_owned())
        );
        assert!(cache.get("0xdeadbeef").is_none());
    }

    #[test]
    fn test_cache_miss_marker_persistence() {
        let mut cache = DiskCache::default();
        cache.insert("0xdeadbeef".to_owned(), CACHE_MISS_MARKER.to_owned());

        let result = cache.get("0xdeadbeef");
        assert_eq!(result, Some(&CACHE_MISS_MARKER.to_owned()));
    }

    #[test]
    fn test_overwrite_existing_entry() {
        let mut cache = DiskCache::default();
        cache.insert("0x12345678".to_owned(), "oldSignature()".to_owned());
        cache.insert("0x12345678".to_owned(), "newSignature(uint256)".to_owned());

        assert_eq!(
            cache.get("0x12345678"),
            Some(&"newSignature(uint256)".to_owned())
        );
    }
}
