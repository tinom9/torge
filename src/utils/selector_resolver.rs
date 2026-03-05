use super::{
    abi_decoder,
    disk_cache::{CacheLookup, DiskCache},
    hex_utils,
};
use reqwest::blocking::Client;
use serde::Deserialize;

/// Default Sourcify API base URL for 4byte mirror.
const DEFAULT_SOURCIFY_4BYTE_URL: &str = "https://api.4byte.sourcify.dev/";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SelectorEntry {
    name: Option<String>,
    #[serde(default)]
    has_verified_contract: bool,
    #[serde(default)]
    filtered: bool,
}

/// Best-effort function selector resolver using Sourcify's 4byte mirror with disk caching.
pub struct SelectorResolver {
    client: Client,
    disk_cache: DiskCache,
    base_url: String,
    enabled: bool,
    warning: Option<String>,
}

impl SelectorResolver {
    pub fn new(client: Client, enabled: bool, base_url: Option<String>) -> Self {
        let mut base_url = base_url
            .or_else(|| std::env::var("SOURCIFY_4BYTE_URL").ok())
            .unwrap_or_else(|| DEFAULT_SOURCIFY_4BYTE_URL.to_owned());
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
        if !self.enabled || hex_utils::require_0x(key).is_none() || key.len() != expected_len {
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

        let Ok(mut body) = resp.json::<serde_json::Value>() else {
            self.disk_cache.insert_transient_miss(key.to_owned());
            return None;
        };

        if !body["ok"].as_bool().unwrap_or(false) {
            self.disk_cache.insert_miss(key.to_owned());
            return None;
        }

        let entries: Vec<SelectorEntry> =
            serde_json::from_value(body["result"][kind][key].take()).unwrap_or_default();

        if entries.is_empty() {
            self.disk_cache.insert_miss(key.to_owned());
            return None;
        }

        let Some(sig) = select_best_entry(&entries, calldata) else {
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
fn select_best_entry(entries: &[SelectorEntry], calldata: Option<&str>) -> Option<String> {
    let name = |e: &SelectorEntry| e.name.clone();
    let verified = |e: &SelectorEntry| e.has_verified_contract;
    let filtered = |e: &SelectorEntry| e.filtered;

    if let Some(calldata) = calldata {
        let decodable =
            |e: &SelectorEntry| name(e).is_some_and(|n| abi_decoder::can_decode(&n, calldata));

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

#[cfg(test)]
mod tests {
    use super::*;

    const DECODABLE_SIG: &str = "transfer(address,uint256)";
    const DECODABLE_SIG_ALT: &str = "approve(address,uint256)";
    const NON_DECODABLE_SIG: &str = "foo(address,address,address)";
    const CALLDATA: &str = "0xa9059cbb000000000000000000000000deadbeefdeadbeefdeadbeefdeadbeefdeadbeef00000000000000000000000000000000000000000000000000000000000003e8";

    fn entry(name: &str, verified: bool, filtered: bool) -> SelectorEntry {
        SelectorEntry {
            name: Some(name.to_owned()),
            has_verified_contract: verified,
            filtered,
        }
    }

    #[test]
    fn test_select_best_entry_single_tiers() {
        let calldata = Some(CALLDATA);

        let entries = vec![entry(DECODABLE_SIG, true, false)];
        assert_eq!(
            select_best_entry(&entries, calldata),
            Some(DECODABLE_SIG.to_string())
        );

        let entries = vec![entry(DECODABLE_SIG, true, true)];
        assert_eq!(
            select_best_entry(&entries, calldata),
            Some(DECODABLE_SIG.to_string())
        );

        let entries = vec![entry(DECODABLE_SIG, false, false)];
        assert_eq!(
            select_best_entry(&entries, calldata),
            Some(DECODABLE_SIG.to_string())
        );

        let entries = vec![entry(DECODABLE_SIG, false, true)];
        assert_eq!(
            select_best_entry(&entries, calldata),
            Some(DECODABLE_SIG.to_string())
        );
    }

    #[test]
    fn test_select_best_entry_non_decodable_fallback() {
        let entries = vec![entry(NON_DECODABLE_SIG, true, false)];
        assert_eq!(
            select_best_entry(&entries, Some(CALLDATA)),
            Some(NON_DECODABLE_SIG.to_string()),
        );
    }

    #[test]
    fn test_select_best_entry_missing_name() {
        let entries = vec![SelectorEntry {
            name: None,
            has_verified_contract: true,
            filtered: false,
        }];
        assert_eq!(select_best_entry(&entries, Some(CALLDATA)), None);
    }

    #[test]
    fn test_select_best_entry_priority_order() {
        let calldata = Some(CALLDATA);

        let entries = vec![
            entry(DECODABLE_SIG_ALT, true, true),
            entry(DECODABLE_SIG, true, false),
        ];
        assert_eq!(
            select_best_entry(&entries, calldata),
            Some(DECODABLE_SIG.to_string())
        );

        let entries = vec![
            entry(DECODABLE_SIG_ALT, false, false),
            entry(DECODABLE_SIG, true, true),
        ];
        assert_eq!(
            select_best_entry(&entries, calldata),
            Some(DECODABLE_SIG.to_string())
        );

        let entries = vec![
            entry(DECODABLE_SIG_ALT, false, true),
            entry(DECODABLE_SIG, false, false),
        ];
        assert_eq!(
            select_best_entry(&entries, calldata),
            Some(DECODABLE_SIG.to_string())
        );
    }

    #[test]
    fn test_select_best_entry_decodable_beats_non_decodable() {
        let entries = vec![
            entry(NON_DECODABLE_SIG, true, false),
            entry(DECODABLE_SIG, false, true),
        ];
        assert_eq!(
            select_best_entry(&entries, Some(CALLDATA)),
            Some(DECODABLE_SIG.to_string()),
        );
    }

    #[test]
    fn test_select_best_entry_all_non_decodable() {
        let entries = vec![
            entry(NON_DECODABLE_SIG, true, false),
            entry("bar(address,address,address)", false, true),
        ];
        assert_eq!(
            select_best_entry(&entries, Some(CALLDATA)),
            Some(NON_DECODABLE_SIG.to_string()),
        );
    }

    #[test]
    fn test_select_best_entry_empty() {
        let entries: Vec<SelectorEntry> = vec![];
        assert_eq!(select_best_entry(&entries, Some(CALLDATA)), None);
    }

    #[test]
    fn test_select_best_entry_event_single() {
        let entries = vec![entry("Transfer(address,address,uint256)", true, false)];
        assert_eq!(
            select_best_entry(&entries, None),
            Some("Transfer(address,address,uint256)".to_string()),
        );
    }

    #[test]
    fn test_select_best_entry_event_no_name() {
        let entries = vec![SelectorEntry {
            name: None,
            has_verified_contract: true,
            filtered: false,
        }];
        assert_eq!(select_best_entry(&entries, None), None);
    }

    #[test]
    fn test_select_best_entry_event_multiple() {
        let entries = vec![
            entry("Transfer(address,address,uint256)", true, false),
            entry("Approval(address,address,uint256)", true, false),
        ];
        assert_eq!(
            select_best_entry(&entries, None),
            Some("Transfer(address,address,uint256)".to_string()),
        );
    }

    #[test]
    fn test_select_best_entry_event_empty() {
        let entries: Vec<SelectorEntry> = vec![];
        assert_eq!(select_best_entry(&entries, None), None);
    }
}
