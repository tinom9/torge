use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};
use thiserror::Error;

const CACHE_MISS_MARKER: &str = "<UNKNOWN>";

pub const SELECTOR_CACHE: &str = "selectors";
pub const CONTRACT_CACHE: &str = "contracts";
pub const ALL_CACHE_KINDS: &[&str] = &[SELECTOR_CACHE, CONTRACT_CACHE];

/// Result of looking up a key in the cache.
#[derive(Debug)]
pub enum CacheLookup<'a> {
    /// Key found and resolved to a value.
    Hit(&'a str),
    /// Key was previously looked up but not found upstream.
    Miss,
    /// Key not present in cache — requires a network lookup.
    NotCached,
}

#[derive(Debug, Error)]
pub enum CacheError {
    #[error("could not determine cache directory")]
    NoCacheDir,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Reusable disk-backed key-value cache.
///
/// Each cache instance is identified by a `kind` (e.g. `"selectors"`, `"contracts"`).
/// The kind determines both the file name (`{kind}.json`) and the top-level JSON key
/// used for serialization, keeping backward compatibility with existing cache files.
pub struct DiskCache {
    entries: HashMap<String, String>,
    transient_misses: HashSet<String>,
    kind: String,
    dirty: bool,
    disabled: bool,
}

impl DiskCache {
    /// Get the path to a cache file for the given `kind`.
    ///
    /// Respects `XDG_CACHE_HOME` on all platforms (not just Linux) so that
    /// integration tests using a temp directory work on macOS/Windows too.
    pub fn cache_path(kind: &str) -> Option<PathBuf> {
        let base = std::env::var("XDG_CACHE_HOME")
            .ok()
            .map(PathBuf::from)
            .or_else(dirs::cache_dir)?;
        Some(base.join("torge").join(format!("{kind}.json")))
    }

    pub fn load(kind: &str) -> Self {
        let disabled = std::env::var("TORGE_DISABLE_CACHE").is_ok();
        let empty = Self {
            entries: HashMap::new(),
            transient_misses: HashSet::new(),
            kind: kind.to_owned(),
            dirty: false,
            disabled,
        };

        if disabled {
            return empty;
        }

        let Some(path) = Self::cache_path(kind) else {
            return empty;
        };

        let entries = Self::read_entries(&path, kind).unwrap_or_default();

        Self {
            entries,
            transient_misses: HashSet::new(),
            kind: kind.to_owned(),
            dirty: false,
            disabled: false,
        }
    }

    fn read_entries(path: &Path, kind: &str) -> Result<HashMap<String, String>, CacheError> {
        let contents = fs::read_to_string(path)?;
        let mut raw: serde_json::Value = serde_json::from_str(&contents)?;
        Ok(raw
            .as_object_mut()
            .and_then(|m| m.remove(kind))
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default())
    }

    pub fn save(&self) {
        if self.disabled {
            return;
        }

        let Some(path) = Self::cache_path(&self.kind) else {
            return;
        };

        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        let wrapper = serde_json::json!({ &self.kind: &self.entries });
        let contents = match serde_json::to_string_pretty(&wrapper) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("warning: failed to serialize {} cache: {e}", self.kind);
                return;
            }
        };

        let tmp_path = path.with_extension("json.tmp");
        if let Err(e) = fs::write(&tmp_path, contents) {
            eprintln!("warning: failed to write {} cache: {e}", self.kind);
            return;
        }
        if let Err(e) = fs::rename(&tmp_path, &path) {
            eprintln!("warning: failed to rename {} cache: {e}", self.kind);
        }
    }

    /// Look up a key, distinguishing between a resolved hit, a cached miss, and not-in-cache.
    pub fn lookup(&self, key: &str) -> CacheLookup<'_> {
        if self.transient_misses.contains(key) {
            return CacheLookup::Miss;
        }
        match self.entries.get(key) {
            Some(v) if v == CACHE_MISS_MARKER => CacheLookup::Miss,
            Some(v) => CacheLookup::Hit(v),
            None => CacheLookup::NotCached,
        }
    }

    pub fn insert(&mut self, key: String, value: String) {
        self.entries.insert(key, value);
        self.dirty = true;
    }

    pub fn insert_miss(&mut self, key: String) {
        self.insert(key, CACHE_MISS_MARKER.to_owned());
    }

    /// Record a miss that should not be persisted to disk (e.g. transient network errors).
    pub fn insert_transient_miss(&mut self, key: String) {
        self.transient_misses.insert(key);
    }

    /// Remove all unknown (unresolved) entries from the persisted cache.
    /// Returns `(kept, removed)` counts.
    pub fn remove_unknown(kind: &str) -> Result<(usize, usize), CacheError> {
        let path = Self::cache_path(kind).ok_or(CacheError::NoCacheDir)?;
        if !path.exists() {
            return Ok((0, 0));
        }

        let mut entries = Self::read_entries(&path, kind)?;

        let before = entries.len();
        entries.retain(|_, v| v != CACHE_MISS_MARKER);
        let after = entries.len();

        let wrapper = serde_json::json!({ kind: &entries });
        let json = serde_json::to_string_pretty(&wrapper)?;
        let tmp_path = path.with_extension("json.tmp");
        fs::write(&tmp_path, json)?;
        fs::rename(&tmp_path, &path)?;

        Ok((after, before - after))
    }
}

impl Drop for DiskCache {
    fn drop(&mut self) {
        if self.dirty {
            self.save();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_cache() -> DiskCache {
        DiskCache {
            entries: HashMap::new(),
            transient_misses: HashSet::new(),
            kind: "test".to_owned(),
            dirty: false,
            disabled: true,
        }
    }

    #[test]
    fn test_insert_and_lookup() {
        let mut cache = empty_cache();
        cache.insert(
            "0x12345678".to_owned(),
            "transfer(address,uint256)".to_owned(),
        );

        match cache.lookup("0x12345678") {
            CacheLookup::Hit(sig) => assert_eq!(sig, "transfer(address,uint256)"),
            other => panic!("expected Hit, got {other:?}"),
        }
        assert!(matches!(cache.lookup("0xdeadbeef"), CacheLookup::NotCached));
    }

    #[test]
    fn test_insert_miss_and_lookup() {
        let mut cache = empty_cache();
        cache.insert_miss("0xdeadbeef".to_owned());
        assert!(matches!(cache.lookup("0xdeadbeef"), CacheLookup::Miss));
    }

    #[test]
    fn test_transient_miss() {
        let mut cache = empty_cache();
        cache.insert_transient_miss("0xdeadbeef".to_owned());
        assert!(matches!(cache.lookup("0xdeadbeef"), CacheLookup::Miss));
        assert!(!cache.entries.contains_key("0xdeadbeef"));
        assert!(cache.transient_misses.contains("0xdeadbeef"));
    }
}
