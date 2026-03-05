use super::disk_cache::{CacheLookup, DiskCache, CONTRACT_CACHE};
use super::hex_utils;
use reqwest::blocking::Client;
use serde::Deserialize;

const DEFAULT_SOURCIFY_SERVER_URL: &str = "https://sourcify.dev/server/";

#[derive(Deserialize)]
struct ContractInfo {
    compilation: Option<Compilation>,
}

#[derive(Deserialize)]
struct Compilation {
    name: Option<String>,
}

/// Best-effort contract name resolver using Sourcify's v2 API with disk caching.
pub struct ContractResolver {
    client: Client,
    disk_cache: DiskCache,
    base_url: String,
    chain_id: Option<String>,
    enabled: bool,
    warning: Option<String>,
}

impl ContractResolver {
    pub fn new(
        client: Client,
        chain_id: Option<String>,
        enabled: bool,
        base_url: Option<String>,
    ) -> Self {
        let mut base_url = base_url
            .or_else(|| std::env::var("SOURCIFY_SERVER_URL").ok())
            .unwrap_or_else(|| DEFAULT_SOURCIFY_SERVER_URL.to_owned());
        if !base_url.ends_with('/') {
            base_url.push('/');
        }
        Self {
            client,
            disk_cache: DiskCache::load(CONTRACT_CACHE),
            base_url,
            chain_id,
            enabled,
            warning: None,
        }
    }

    pub fn take_warning(&mut self) -> Option<String> {
        self.warning.take()
    }

    /// Resolve a contract address to its name via Sourcify's v2 contract lookup.
    pub fn resolve(&mut self, address: &str) -> Option<String> {
        if !self.enabled {
            return None;
        }
        let chain_id = self.chain_id.as_deref()?;
        if !hex_utils::is_valid_address(address) {
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

        let resp = match self.client.get(&url).send() {
            Ok(r) if r.status().is_success() => r,
            Ok(r) if r.status() == reqwest::StatusCode::NOT_FOUND => {
                self.disk_cache.insert_miss(cache_key);
                return None;
            }
            Ok(_) | Err(_) => {
                if self.warning.is_none() {
                    self.warning = Some(format!(
                        "sourcify contract lookup failed for {address}, results may be incomplete"
                    ));
                }
                self.disk_cache.insert_transient_miss(cache_key);
                return None;
            }
        };

        let Ok(info) = resp.json::<ContractInfo>() else {
            self.disk_cache.insert_transient_miss(cache_key);
            return None;
        };

        if let Some(name) = info
            .compilation
            .and_then(|c| c.name)
            .filter(|n| !n.is_empty())
        {
            self.disk_cache.insert(cache_key, name.clone());
            Some(name)
        } else {
            self.disk_cache.insert_miss(cache_key);
            None
        }
    }
}
