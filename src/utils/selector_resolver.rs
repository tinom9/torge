use super::{
    abi_decoder,
    disk_cache::{DiskCache, CACHE_MISS_MARKER},
};
use reqwest::blocking::Client;
use serde::Deserialize;
use std::collections::HashMap;

/// Default Sourcify API base URL for 4byte mirror.
const DEFAULT_SOURCIFY_URL: &str = "https://api.4byte.sourcify.dev/";

/// Best-effort function selector resolver using Sourcify's 4byte mirror with disk caching.
pub struct SelectorResolver<'a> {
    client: &'a Client,
    memory_cache: HashMap<String, String>,
    disk_cache: DiskCache,
    enabled: bool,
    cache_dirty: bool,
}

impl<'a> SelectorResolver<'a> {
    pub fn new(client: &'a Client, enabled: bool) -> Self {
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

    pub fn resolve(&mut self, selector: &str, calldata: Option<&str>) -> Option<String> {
        if !self.enabled {
            return None;
        }

        if !selector.starts_with("0x") || selector.len() != 10 {
            return None;
        }

        if let Some(sig) = self.memory_cache.get(selector) {
            return (sig != CACHE_MISS_MARKER).then(|| sig.clone());
        }

        if let Some(sig) = self.disk_cache.get(selector) {
            self.memory_cache.insert(selector.to_string(), sig.clone());
            return (sig != CACHE_MISS_MARKER).then(|| sig.clone());
        }
        let base_url =
            std::env::var("SOURCIFY_URL").unwrap_or_else(|_| DEFAULT_SOURCIFY_URL.to_string());
        let url =
            format!("{base_url}signature-database/v1/lookup?function={selector}&filter=false");

        let resp = match self
            .client
            .get(&url)
            .send()
            .ok()
            .and_then(|r| r.error_for_status().ok())
        {
            Some(r) => r,
            None => {
                self.cache_miss(selector);
                return None;
            }
        };

        #[derive(Deserialize)]
        struct SourcifyResponse {
            ok: bool,
            result: SourcifyResult,
        }

        #[derive(Deserialize)]
        struct SourcifyResult {
            function: HashMap<String, Vec<SourcifyEntry>>,
        }

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct SourcifyEntry {
            name: String,
            filtered: bool,
            has_verified_contract: bool,
        }

        let response: SourcifyResponse = match resp.json().ok() {
            Some(r) => r,
            None => {
                self.cache_miss(selector);
                return None;
            }
        };

        if !response.ok {
            self.cache_miss(selector);
            return None;
        }

        let entries = match response.result.function.get(selector) {
            Some(e) if !e.is_empty() => e,
            _ => {
                self.cache_miss(selector);
                return None;
            }
        };

        let sig = if let Some(calldata) = calldata {
            entries
                .iter()
                .filter(|e| e.has_verified_contract && !e.filtered)
                .find(|e| can_decode(&e.name, calldata))
                .or_else(|| {
                    entries
                        .iter()
                        .filter(|e| e.has_verified_contract)
                        .find(|e| can_decode(&e.name, calldata))
                })
                .or_else(|| {
                    entries
                        .iter()
                        .filter(|e| !e.filtered)
                        .find(|e| can_decode(&e.name, calldata))
                })
                .or_else(|| entries.iter().find(|e| can_decode(&e.name, calldata)))
                .or_else(|| entries.first())
                .map(|e| e.name.clone())?
        } else {
            entries
                .iter()
                .find(|e| e.has_verified_contract && !e.filtered)
                .or_else(|| entries.iter().find(|e| e.has_verified_contract))
                .or_else(|| entries.iter().find(|e| !e.filtered))
                .or_else(|| entries.first())
                .map(|e| e.name.clone())?
        };

        self.cache_signature(selector, &sig);
        Some(sig)
    }

    fn cache_miss(&mut self, selector: &str) {
        self.memory_cache
            .insert(selector.to_owned(), CACHE_MISS_MARKER.to_owned());
        self.disk_cache
            .insert(selector.to_owned(), CACHE_MISS_MARKER.to_owned());
        self.cache_dirty = true;
    }

    fn cache_signature(&mut self, selector: &str, signature: &str) {
        self.memory_cache
            .insert(selector.to_owned(), signature.to_owned());
        self.disk_cache
            .insert(selector.to_owned(), signature.to_owned());
        self.cache_dirty = true;
    }
}

impl<'a> Drop for SelectorResolver<'a> {
    fn drop(&mut self) {
        if self.cache_dirty {
            self.disk_cache.save();
        }
    }
}

fn can_decode(signature: &str, calldata: &str) -> bool {
    abi_decoder::can_decode(signature, calldata)
}
