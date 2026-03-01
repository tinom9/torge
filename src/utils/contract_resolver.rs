use super::disk_cache::{CacheLookup, DiskCache};
use reqwest::blocking::Client;

const DEFAULT_SOURCIFY_SERVER_URL: &str = "https://sourcify.dev/server/";

/// Best-effort contract name resolver using Sourcify's v2 API with disk caching.
pub struct ContractResolver {
    client: Client,
    disk_cache: DiskCache,
    base_url: String,
    chain_id: Option<String>,
    enabled: bool,
}

impl ContractResolver {
    pub fn new(client: Client, chain_id: Option<String>, enabled: bool) -> Self {
        let mut base_url = std::env::var("SOURCIFY_SERVER_URL")
            .unwrap_or_else(|_| DEFAULT_SOURCIFY_SERVER_URL.to_owned());
        if !base_url.ends_with('/') {
            base_url.push('/');
        }
        Self {
            client,
            disk_cache: DiskCache::load("contracts"),
            base_url,
            chain_id,
            enabled,
        }
    }

    /// Resolve a contract address to its name via Sourcify's v2 contract lookup.
    pub fn resolve(&mut self, address: &str) -> Option<String> {
        if !self.enabled {
            return None;
        }
        let chain_id = self.chain_id.as_deref()?;
        if !address.starts_with("0x") || address.len() != 42 {
            return None;
        }

        let address = address.to_lowercase();
        let cache_key = format!("{chain_id}:{address}");

        match self.disk_cache.lookup(&cache_key) {
            CacheLookup::Hit(name) => return Some(name.to_owned()),
            CacheLookup::Miss => return None,
            CacheLookup::NotCached => {}
        }

        let url = format!(
            "{}v2/contract/{chain_id}/{address}?fields=compilation.name",
            self.base_url
        );

        let Some(resp) = self
            .client
            .get(&url)
            .send()
            .ok()
            .and_then(|r| r.error_for_status().ok())
        else {
            self.disk_cache.insert_miss(cache_key);
            return None;
        };

        let Some(body) = resp.json::<serde_json::Value>().ok() else {
            self.disk_cache.insert_miss(cache_key);
            return None;
        };

        match body["compilation"]["name"].as_str() {
            Some(name) if !name.is_empty() => {
                let name = name.to_owned();
                self.disk_cache.insert(cache_key, name.clone());
                Some(name)
            }
            _ => {
                self.disk_cache.insert_miss(cache_key);
                None
            }
        }
    }
}
