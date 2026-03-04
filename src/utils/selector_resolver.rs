use super::{
    abi_decoder,
    disk_cache::{CacheLookup, DiskCache},
};
use reqwest::blocking::Client;

/// Default Sourcify API base URL for 4byte mirror.
const DEFAULT_SOURCIFY_4BYTE_URL: &str = "https://api.4byte.sourcify.dev/";

/// Best-effort function selector resolver using Sourcify's 4byte mirror with disk caching.
pub struct SelectorResolver {
    client: Client,
    disk_cache: DiskCache,
    base_url: String,
    enabled: bool,
    warning: Option<String>,
}

impl SelectorResolver {
    pub fn new(client: Client, enabled: bool) -> Self {
        let mut base_url = std::env::var("SOURCIFY_4BYTE_URL")
            .unwrap_or_else(|_| DEFAULT_SOURCIFY_4BYTE_URL.to_owned());
        if !base_url.ends_with('/') {
            base_url.push('/');
        }
        Self {
            client,
            disk_cache: DiskCache::load("selectors"),
            base_url,
            enabled,
            warning: None,
        }
    }

    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn take_warning(&mut self) -> Option<String> {
        self.warning.take()
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

        match self.disk_cache.lookup(key) {
            CacheLookup::Hit(sig) => return Some(sig.to_owned()),
            CacheLookup::Miss => return None,
            CacheLookup::NotCached => {}
        }

        let url = format!(
            "{}signature-database/v1/lookup?{kind}={key}&filter=false",
            self.base_url
        );

        let resp = match self.client.get(&url).send() {
            Ok(r) if r.status().is_success() => r,
            Ok(_) | Err(_) => {
                if self.warning.is_none() {
                    self.warning = Some(format!(
                        "sourcify selector lookup failed for {key}, results may be incomplete"
                    ));
                }
                self.disk_cache.insert_transient_miss(key.to_owned());
                return None;
            }
        };

        let Some(body) = resp.json::<serde_json::Value>().ok() else {
            self.disk_cache.insert_transient_miss(key.to_owned());
            return None;
        };

        if !body["ok"].as_bool().unwrap_or(false) {
            self.disk_cache.insert_miss(key.to_owned());
            return None;
        }

        let entries = match body["result"][kind][key].as_array() {
            Some(e) if !e.is_empty() => e,
            _ => {
                self.disk_cache.insert_miss(key.to_owned());
                return None;
            }
        };

        let Some(sig) = select_best_entry(entries, calldata) else {
            self.disk_cache.insert_miss(key.to_owned());
            return None;
        };
        self.disk_cache.insert(key.to_owned(), sig.clone());
        Some(sig)
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
