use crate::utils::{
    color::Palette,
    contract_resolver::ContractResolver,
    hex_utils, rpc_url,
    selector_resolver::SelectorResolver,
    storage_diff::{self, PrestateDiff},
    trace_renderer::{self, CallTrace},
};
use clap::Parser;
use reqwest::blocking::Client;
use serde::Deserialize;
use std::time::Duration;
use thiserror::Error;

/// Common trace output options shared by `tx` and `call`.
#[derive(Parser, Debug)]
#[allow(clippy::struct_excessive_bools)] // CLI flags are naturally boolean
pub struct TraceOpts {
    /// Ethereum JSON-RPC URL or Foundry alias (e.g. `http://localhost:8545` or `ethereum`).
    ///
    /// If a URL is provided (starts with `http://` or `https://`), it will be used directly.
    /// Otherwise, the tool will look for a matching alias in foundry.toml's [`rpc_endpoints`].
    /// If not provided, will read from the `RPC_URL` or `ETH_RPC_URL` environment variables.
    #[arg(short, long)]
    pub rpc_url: Option<String>,

    /// Resolve 4-byte selectors via Sourcify's 4byte mirror (best-effort).
    ///
    /// When enabled, the tool will try to map 0x12345678 selectors to
    /// text signatures like `transfer(address,uint256)`.
    #[arg(long)]
    pub resolve_selectors: bool,

    /// Resolve contract addresses to names via Sourcify's v2 API (best-effort).
    ///
    /// When enabled, the tool will try to replace contract addresses with
    /// their verified contract names (e.g. `TetherToken` instead of `0xdAC1…`).
    /// Requires an `eth_chainId` RPC call to determine the network.
    #[arg(long)]
    pub resolve_contracts: bool,

    /// Decode and display call arguments using the resolved function signature.
    ///
    /// Must be provided with `--resolve-selectors`.
    #[arg(long)]
    pub include_args: bool,

    /// Also print the raw calldata hex for each call (for manual inspection).
    #[arg(long)]
    pub include_calldata: bool,

    /// Include event logs (emits) in the trace output.
    ///
    /// Without `--resolve-selectors`, shows raw topic0 and full data.
    /// With `--resolve-selectors`, resolves event names and decodes parameters.
    #[arg(long)]
    pub include_logs: bool,

    /// Include storage changes (state diffs) after the trace.
    ///
    /// Requires an additional RPC call using `prestateTracer` in diff mode.
    #[arg(long)]
    pub include_storage: bool,

    /// Disable system proxy for RPC requests.
    #[arg(long)]
    pub no_proxy: bool,

    /// Disable colored output (auto-detected when stdout is not a terminal).
    #[arg(long)]
    pub no_color: bool,
}

#[derive(Debug, Error)]
pub enum TraceError {
    #[error("missing RPC url, pass --rpc-url or set RPC_URL / ETH_RPC_URL")]
    MissingRpcUrl,

    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("rpc error: {0} ({1})")]
    Rpc(String, i64),

    #[error("decode error: {0}")]
    Decode(String),

    #[error("--include-args requires --resolve-selectors to be enabled")]
    IncludeArgsRequiresResolveSelectors,

    #[error("rpc url error: {0}")]
    RpcUrl(#[from] rpc_url::RpcUrlError),

    #[error("invalid value: {0}")]
    InvalidValue(String),

    #[error("invalid input: {0}")]
    InvalidInput(String),
}

#[derive(Debug, Deserialize)]
struct RpcResponse<T> {
    result: Option<T>,
    error: Option<RpcError>,
}

#[derive(Debug, Deserialize)]
struct RpcError {
    code: i64,
    message: String,
}

/// Create a pre-configured HTTP client for RPC calls.
fn create_client(no_proxy: bool) -> Result<Client, TraceError> {
    let mut builder = Client::builder().timeout(Duration::from_secs(60));
    if no_proxy {
        builder = builder.no_proxy();
    }
    Ok(builder.build()?)
}

/// Resolve RPC URL from an optional user-provided value or env vars.
fn resolve_rpc_url(url_or_alias: Option<String>) -> Result<String, TraceError> {
    match url_or_alias {
        Some(val) => Ok(rpc_url::resolve(&val)?),
        None => std::env::var("RPC_URL")
            .or_else(|_| std::env::var("ETH_RPC_URL"))
            .map_err(|_| TraceError::MissingRpcUrl),
    }
}

/// Validate that a string is a 0x-prefixed Ethereum address (40 hex chars).
pub fn validate_address(addr: &str, field: &str) -> Result<(), TraceError> {
    if !hex_utils::is_valid_address(addr) {
        return Err(TraceError::InvalidInput(format!(
            "{field}: expected 0x-prefixed 40-char hex address"
        )));
    }
    Ok(())
}

/// Validate that a string is a 0x-prefixed transaction hash (64 hex chars).
pub fn validate_tx_hash(hash: &str, field: &str) -> Result<(), TraceError> {
    if !hex_utils::is_valid_tx_hash(hash) {
        return Err(TraceError::InvalidInput(format!(
            "{field}: expected 0x-prefixed 64-char hex hash"
        )));
    }
    Ok(())
}

/// Validate that a string is 0x-prefixed hex data with an even number of hex chars.
pub fn validate_hex(data: &str, field: &str) -> Result<(), TraceError> {
    let hex = hex_utils::require_0x(data)
        .ok_or_else(|| TraceError::InvalidInput(format!("{field}: missing 0x prefix")))?;
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(TraceError::InvalidInput(format!(
            "{field}: invalid hex characters"
        )));
    }
    if hex.len() % 2 != 0 {
        return Err(TraceError::InvalidInput(format!(
            "{field}: odd number of hex characters (must be full bytes)"
        )));
    }
    Ok(())
}

/// Build a JSON-RPC request envelope.
pub fn rpc_payload(id: u32, method: &str, params: serde_json::Value) -> serde_json::Value {
    let mut v = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
    });
    v["params"] = params;
    v
}

pub fn call_tracer_config(include_logs: bool) -> serde_json::Value {
    serde_json::json!({
        "tracer": "callTracer",
        "tracerConfig": {
            "onlyTopCall": false,
            "withLog": include_logs,
        }
    })
}

pub fn prestate_tracer_config() -> serde_json::Value {
    serde_json::json!({
        "tracer": "prestateTracer",
        "tracerConfig": {
            "diffMode": true
        }
    })
}

/// Fetch the chain ID from the RPC node as a decimal string (e.g. `"1"`).
fn fetch_chain_id(client: &Client, rpc_url: &str) -> Result<String, TraceError> {
    let payload = rpc_payload(1, "eth_chainId", serde_json::json!([]));
    let resp = client.post(rpc_url).json(&payload).send()?;
    let hex_str: String = parse_rpc_response(resp)?;
    parse_chain_id_hex(&hex_str)
}

/// Parse a hex chain ID (e.g. `"0x1"`, `"0xa"`) into a decimal string.
fn parse_chain_id_hex(hex_str: &str) -> Result<String, TraceError> {
    let stripped = hex_utils::require_0x(hex_str)
        .ok_or_else(|| TraceError::Decode(format!("eth_chainId: invalid hex '{hex_str}'")))?;

    u64::from_str_radix(stripped, 16)
        .map(|n| n.to_string())
        .map_err(|_| TraceError::Decode(format!("eth_chainId: invalid hex '{hex_str}'")))
}

/// Send an RPC payload, parse the trace response, and print it.
///
/// When `prestate_payload` is provided, a second RPC call is made using
/// `prestateTracer` in diff mode to display storage changes after the trace.
pub fn execute_and_print(
    payload: &serde_json::Value,
    prestate_payload: Option<&serde_json::Value>,
    opts: TraceOpts,
) -> Result<(), TraceError> {
    if opts.include_args && !opts.resolve_selectors {
        return Err(TraceError::IncludeArgsRequiresResolveSelectors);
    }

    let rpc_url = resolve_rpc_url(opts.rpc_url)?;
    let client = create_client(opts.no_proxy)?;

    let chain_id = if opts.resolve_contracts {
        match fetch_chain_id(&client, &rpc_url) {
            Ok(id) => Some(id),
            Err(e) => {
                eprintln!("warning: could not fetch chain ID, contract resolution disabled: {e}");
                None
            }
        }
    } else {
        None
    };

    let resp = client.post(&rpc_url).json(payload).send()?;
    let call_trace: CallTrace = parse_rpc_response(resp)?;

    let prestate_diff: Option<PrestateDiff> = if let Some(ps) = prestate_payload {
        let result = client
            .post(&rpc_url)
            .json(ps)
            .send()
            .map_err(TraceError::from)
            .and_then(parse_rpc_response);
        match result {
            Ok(diff) => Some(diff),
            Err(e) => {
                eprintln!("warning: storage diff unavailable: {e}");
                None
            }
        }
    } else {
        None
    };

    let palette = if opts.no_color {
        Palette::new(false)
    } else {
        Palette::auto()
    };

    let mut selector_resolver = SelectorResolver::new(client.clone(), opts.resolve_selectors, None);
    let mut contract_resolver =
        ContractResolver::new(client, chain_id, opts.resolve_contracts, None);
    trace_renderer::print_trace(
        &call_trace,
        &mut selector_resolver,
        &mut contract_resolver,
        opts.include_args,
        opts.include_calldata,
        opts.include_logs,
        palette,
    );

    if let Some(diff) = &prestate_diff {
        storage_diff::print_storage_diff(diff, &mut contract_resolver, palette);
    }

    let warnings: Vec<String> = [
        selector_resolver.take_warning(),
        contract_resolver.take_warning(),
    ]
    .into_iter()
    .flatten()
    .collect();

    if !warnings.is_empty() {
        eprintln!();
        for w in warnings {
            eprintln!("{}", palette.yellow(&format!("warning: {w}")));
        }
    }

    Ok(())
}

/// Parse the RPC response, returning the result or an error.
fn parse_rpc_response<T: serde::de::DeserializeOwned>(
    resp: reqwest::blocking::Response,
) -> Result<T, TraceError> {
    let status = resp.status();
    let body = resp.text()?;

    if !status.is_success() {
        return Err(TraceError::Decode(format!(
            "HTTP {status}: {}",
            body.chars().take(200).collect::<String>()
        )));
    }

    let rpc_resp: RpcResponse<T> = serde_json::from_str(&body)
        .map_err(|e| TraceError::Decode(format!("invalid JSON: {e}")))?;

    if let Some(err) = rpc_resp.error {
        return Err(TraceError::Rpc(err.message, err.code));
    }

    rpc_resp
        .result
        .ok_or_else(|| TraceError::Decode("missing result field in RPC response".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_address() {
        assert!(validate_address("0xdAC17F958D2ee523a2206206994597C13D831ec7", "to").is_ok());
        assert!(validate_address("0x0000000000000000000000000000000000000000", "to").is_ok());
        assert!(validate_address("dAC17F958D2ee523a2206206994597C13D831ec7", "to").is_err());
        assert!(validate_address("0x1234", "to").is_err());
        assert!(validate_address("0xZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ", "to").is_err());
    }

    #[test]
    fn test_validate_hex() {
        assert!(validate_hex("0xa9059cbb", "data").is_ok());
        assert!(validate_hex("0x", "data").is_ok());
        assert!(validate_hex("a9059cbb", "data").is_err());
        assert!(validate_hex("0xGGGG", "data").is_err());
        assert!(validate_hex("0xabc", "data").is_err()); // odd length
        assert!(validate_hex("0xab", "data").is_ok());
    }

    #[test]
    fn test_validate_tx_hash() {
        assert!(validate_tx_hash(
            "0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
            "hash"
        )
        .is_ok());
        assert!(validate_tx_hash("0x1234", "hash").is_err());
        assert!(validate_tx_hash(
            "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
            "hash"
        )
        .is_err());
    }

    #[test]
    fn test_validate_address_uppercase_prefix() {
        assert!(validate_address("0XdAC17F958D2ee523a2206206994597C13D831ec7", "to").is_ok());
    }

    #[test]
    fn test_validate_hex_uppercase_prefix() {
        assert!(validate_hex("0Xa9059cbb", "data").is_ok());
    }

    #[test]
    fn test_parse_chain_id_hex() {
        assert_eq!(parse_chain_id_hex("0x1").unwrap(), "1");
        assert_eq!(parse_chain_id_hex("0xa").unwrap(), "10");
        assert_eq!(parse_chain_id_hex("0xa4b1").unwrap(), "42161");
        assert_eq!(parse_chain_id_hex("0x89").unwrap(), "137");
        assert!(parse_chain_id_hex("1").is_err());
        assert!(parse_chain_id_hex("0xZZ").is_err());
    }
}
