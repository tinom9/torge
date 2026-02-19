use super::{
    abi_decoder,
    disk_cache::{DiskCache, CACHE_MISS_MARKER},
};
use reqwest::blocking::Client;
use std::collections::HashMap;

/// Default Sourcify API base URL for 4byte mirror.
const DEFAULT_SOURCIFY_URL: &str = "https://api.4byte.sourcify.dev/";

/// Best-effort function selector resolver using Sourcify's 4byte mirror with disk caching.
pub struct SelectorResolver {
    client: Client,
    memory_cache: HashMap<String, String>,
    disk_cache: DiskCache,
    enabled: bool,
    cache_dirty: bool,
}

impl SelectorResolver {
    pub fn new(client: Client, enabled: bool) -> Self {
        Self {
            client,
            memory_cache: HashMap::new(),
            disk_cache: DiskCache::load(),
            enabled,
            cache_dirty: false,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Resolve a 4-byte function selector to a text signature.
    pub fn resolve(&mut self, selector: &str, calldata: Option<&str>) -> Option<String> {
        self.lookup(selector, "function", 10, calldata)
    }

    /// Resolve an event topic0 hash to an event signature.
    pub fn resolve_event(&mut self, topic0: &str) -> Option<String> {
        self.lookup(topic0, "event", 66, None)
    }

    /// Unified Sourcify lookup for both function selectors and event topics.
    fn lookup(
        &mut self,
        key: &str,
        kind: &str,
        expected_len: usize,
        calldata: Option<&str>,
    ) -> Option<String> {
        if !self.enabled || !key.starts_with("0x") || key.len() != expected_len {
            return None;
        }

        if let Some(sig) = self.memory_cache.get(key) {
            return (sig != CACHE_MISS_MARKER).then(|| sig.clone());
        }

        if let Some(sig) = self.disk_cache.get(key) {
            self.memory_cache.insert(key.to_owned(), sig.to_owned());
            return (sig != CACHE_MISS_MARKER).then(|| sig.to_owned());
        }

        let base_url =
            std::env::var("SOURCIFY_URL").unwrap_or_else(|_| DEFAULT_SOURCIFY_URL.to_owned());
        let url = format!("{base_url}signature-database/v1/lookup?{kind}={key}&filter=false");

        let Some(resp) = self
            .client
            .get(&url)
            .send()
            .ok()
            .and_then(|r| r.error_for_status().ok())
        else {
            self.cache_miss(key);
            return None;
        };

        let Some(body) = resp.json::<serde_json::Value>().ok() else {
            self.cache_miss(key);
            return None;
        };

        if !body["ok"].as_bool().unwrap_or(false) {
            self.cache_miss(key);
            return None;
        }

        let entries = match body["result"][kind][key].as_array() {
            Some(e) if !e.is_empty() => e,
            _ => {
                self.cache_miss(key);
                return None;
            }
        };

        let sig = select_best_entry(entries, calldata)?;
        self.cache_signature(key, &sig);
        Some(sig)
    }

    fn cache_miss(&mut self, key: &str) {
        self.memory_cache
            .insert(key.to_owned(), CACHE_MISS_MARKER.to_owned());
        self.disk_cache
            .insert(key.to_owned(), CACHE_MISS_MARKER.to_owned());
        self.cache_dirty = true;
    }

    fn cache_signature(&mut self, key: &str, signature: &str) {
        self.memory_cache
            .insert(key.to_owned(), signature.to_owned());
        self.disk_cache.insert(key.to_owned(), signature.to_owned());
        self.cache_dirty = true;
    }
}

impl Drop for SelectorResolver {
    fn drop(&mut self) {
        if self.cache_dirty {
            self.disk_cache.save();
        }
    }
}

/// Select the best entry from a Sourcify response array.
///
/// When `calldata` is provided (function lookups), entries are prioritized by:
/// verified + unfiltered + decodable > verified + decodable > unfiltered + decodable >
/// decodable > first entry.
///
/// When `calldata` is `None` (event lookups), returns the first entry.
fn select_best_entry(entries: &[serde_json::Value], calldata: Option<&str>) -> Option<String> {
    let name = |e: &serde_json::Value| e["name"].as_str().map(str::to_owned);
    let verified = |e: &serde_json::Value| e["hasVerifiedContract"].as_bool().unwrap_or(false);
    let filtered = |e: &serde_json::Value| e["filtered"].as_bool().unwrap_or(false);

    if let Some(calldata) = calldata {
        let decodable =
            |e: &serde_json::Value| name(e).is_some_and(|n| abi_decoder::can_decode(&n, calldata));

        entries
            .iter()
            .filter(|e| verified(e) && !filtered(e))
            .find(|e| decodable(e))
            .or_else(|| {
                entries
                    .iter()
                    .filter(|e| verified(e))
                    .find(|e| decodable(e))
            })
            .or_else(|| {
                entries
                    .iter()
                    .filter(|e| !filtered(e))
                    .find(|e| decodable(e))
            })
            .or_else(|| entries.iter().find(|e| decodable(e)))
            .or_else(|| entries.first())
            .and_then(name)
    } else {
        entries.first().and_then(name)
    }
}
