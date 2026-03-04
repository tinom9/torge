use crate::utils::{
    abi_decoder,
    color::Palette,
    contract_resolver::ContractResolver,
    event_formatter::{print_log, Log},
    hex_utils, precompiles, rpc_url,
    selector_resolver::SelectorResolver,
};
use alloy_dyn_abi::{DynSolType, DynSolValue};
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

/// Result shape for geth-style `callTracer`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallTrace {
    #[serde(rename = "type")]
    pub call_type: Option<String>,
    pub to: Option<String>,
    pub value: Option<String>,
    pub gas_used: Option<String>,
    pub input: Option<String>,
    pub output: Option<String>,
    pub error: Option<String>,
    #[serde(default)]
    pub logs: Vec<Log>,
    #[serde(default)]
    pub calls: Vec<CallTrace>,
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
    let hex = hex_utils::require_0x(addr)
        .ok_or_else(|| TraceError::InvalidInput(format!("{field}: missing 0x prefix")))?;
    if hex.len() != 40 {
        return Err(TraceError::InvalidInput(format!(
            "{field}: expected 40 hex chars, got {}",
            hex.len()
        )));
    }
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(TraceError::InvalidInput(format!(
            "{field}: invalid hex characters"
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

/// Fetch the chain ID from the RPC node as a decimal string (e.g. `"1"`).
fn fetch_chain_id(client: &Client, rpc_url: &str) -> Result<String, TraceError> {
    let payload = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "eth_chainId",
        "params": []
    });

    let resp = client.post(rpc_url).json(&payload).send()?;
    let body: RpcResponse<String> = resp
        .json()
        .map_err(|e| TraceError::Decode(format!("eth_chainId: invalid JSON: {e}")))?;

    if let Some(err) = body.error {
        return Err(TraceError::Rpc(err.message, err.code));
    }

    let hex_str = body
        .result
        .ok_or_else(|| TraceError::Decode("eth_chainId: missing result".into()))?;

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
pub fn execute_and_print(payload: &serde_json::Value, opts: TraceOpts) -> Result<(), TraceError> {
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

    let call_trace = parse_rpc_response(resp)?;

    let palette = if opts.no_color {
        Palette::new(false)
    } else {
        Palette::auto()
    };

    let mut selector_resolver = SelectorResolver::new(client.clone(), opts.resolve_selectors);
    let mut contract_resolver = ContractResolver::new(client, chain_id, opts.resolve_contracts);
    print_trace(
        &call_trace,
        &mut selector_resolver,
        &mut contract_resolver,
        opts.include_args,
        opts.include_calldata,
        opts.include_logs,
        palette,
    );

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
fn parse_rpc_response(resp: reqwest::blocking::Response) -> Result<CallTrace, TraceError> {
    let status = resp.status();
    let body = resp.text()?;

    if !status.is_success() {
        return Err(TraceError::Decode(format!(
            "HTTP {status}: {}",
            body.chars().take(200).collect::<String>()
        )));
    }

    let rpc_resp: RpcResponse<CallTrace> = serde_json::from_str(&body)
        .map_err(|e| TraceError::Decode(format!("invalid JSON: {e}")))?;

    if let Some(err) = rpc_resp.error {
        return Err(TraceError::Rpc(err.message, err.code));
    }

    rpc_resp
        .result
        .ok_or_else(|| TraceError::Decode("missing result field in RPC response".into()))
}

fn print_trace(
    root: &CallTrace,
    resolver: &mut SelectorResolver,
    contract_resolver: &mut ContractResolver,
    include_args: bool,
    include_calldata: bool,
    include_logs: bool,
    palette: Palette,
) {
    println!("Traces:");
    print_call(
        root,
        "",
        true,
        resolver,
        contract_resolver,
        include_args,
        include_calldata,
        include_logs,
        palette,
    );
}

#[allow(
    clippy::too_many_lines,
    clippy::fn_params_excessive_bools,
    clippy::too_many_arguments
)]
fn print_call(
    node: &CallTrace,
    prefix: &str,
    is_last: bool,
    resolver: &mut SelectorResolver,
    contract_resolver: &mut ContractResolver,
    include_args: bool,
    include_calldata: bool,
    include_logs: bool,
    pal: Palette,
) {
    let gas_used = node
        .gas_used
        .as_deref()
        .and_then(hex_utils::parse_hex_u256)
        .unwrap_or_default();

    let to = node.to.as_deref().unwrap_or("?");
    let precompile = precompiles::get_precompile_info(to);

    let value = node
        .value
        .as_deref()
        .and_then(hex_utils::parse_hex_u256)
        .filter(|v| !v.is_zero());

    let mut decoded: Option<Vec<(DynSolType, DynSolValue)>> = None;
    let mut decode_attempted = false;

    let is_create = node
        .call_type
        .as_deref()
        .is_some_and(|t| t.eq_ignore_ascii_case("CREATE") || t.eq_ignore_ascii_case("CREATE2"));

    let resolved_name = if !is_create && precompile.is_none() {
        contract_resolver.resolve(to)
    } else {
        None
    };

    let (display_addr, sig) = if is_create {
        (to, String::new())
    } else if let Some((_name, signature)) = precompile {
        if resolver.is_enabled() {
            if include_args {
                if let Some(input_hex) = node.input.as_deref() {
                    decode_attempted = true;
                    decoded = abi_decoder::decode_precompile_args(signature, input_hex);
                }
            }
            ("Precompiles", signature.to_string())
        } else {
            (to, String::new())
        }
    } else {
        let selector = node.input.as_deref().and_then(extract_selector);

        let sig = if let Some(sel) = selector.as_deref() {
            if let Some(text_sig) = resolver.resolve(sel, node.input.as_deref()) {
                if include_args {
                    if let Some(input_hex) = node.input.as_deref() {
                        decode_attempted = true;
                        decoded = abi_decoder::decode_function_args(&text_sig, input_hex);
                    }
                }
                text_sig
            } else {
                sel.to_string()
            }
        } else if resolver.is_enabled() {
            "fallback()".to_string()
        } else {
            "0x".to_string()
        };

        let addr = resolved_name.as_deref().unwrap_or(to);
        (addr, sig)
    };

    let call_desc = format_call_desc(display_addr, &sig, value, pal);

    let call_type = node.call_type.as_deref().unwrap_or("").to_uppercase();

    let call_type_suffix = match call_type.as_str() {
        "DELEGATECALL" => pal.dim(" [delegatecall]"),
        "STATICCALL" => pal.dim(" [staticcall]"),
        "CALLCODE" => pal.dim(" [callcode]"),
        "CREATE" => pal.dim(" [create]"),
        "CREATE2" => pal.dim(" [create2]"),
        _ if !call_type.is_empty() => pal.dim(" [call]"),
        _ => String::new(),
    };

    let connector = if prefix.is_empty() {
        ""
    } else if is_last {
        "└─ "
    } else {
        "├─ "
    };

    let gas_display = pal.dim(&format!("[{gas_used}]"));
    println!("{prefix}{connector}{gas_display} {call_desc}{call_type_suffix}");

    if let Some(err) = &node.error {
        let reason = node
            .output
            .as_deref()
            .and_then(abi_decoder::decode_revert_reason)
            .or_else(|| {
                node.output
                    .as_deref()
                    .and_then(|o| abi_decoder::decode_custom_revert(o, resolver))
            });
        let err_msg = if let Some(reason) = reason {
            pal.red(&format!("↳ error: {err} — {reason}"))
        } else {
            pal.red(&format!("↳ error: {err}"))
        };
        println!("{prefix}    {err_msg}");
    }

    let child_prefix = format!("{}{}", prefix, if is_last { "    " } else { "│   " });
    let meta_prefix = &child_prefix;

    let logs_count = if include_logs { node.logs.len() } else { 0 };
    let total_items = logs_count + node.calls.len();
    let mut item_idx = 0;

    if include_logs {
        for log in &node.logs {
            let is_last_item = item_idx == total_items - 1;
            print_log(log, &child_prefix, is_last_item, resolver, pal);
            item_idx += 1;
        }
    }

    let mut data_printed = false;

    if let Some(args) = decoded {
        if !args.is_empty() {
            println!("{meta_prefix}{}", pal.dim("args:"));

            if args.len() == 1 {
                if let (DynSolType::Tuple(inner_types), DynSolValue::Tuple(inner_values)) = &args[0]
                {
                    let len = inner_types.len().min(inner_values.len());
                    for i in 0..len {
                        print_arg(meta_prefix, 0, i, &inner_types[i], &inner_values[i], pal);
                    }
                } else {
                    for (i, (ty, value)) in args.iter().enumerate() {
                        print_arg(meta_prefix, 0, i, ty, value, pal);
                    }
                }
            } else {
                for (i, (ty, value)) in args.iter().enumerate() {
                    print_arg(meta_prefix, 0, i, ty, value, pal);
                }
            }
        }
    } else if decode_attempted {
        if let Some(input) = &node.input {
            let s = hex_utils::strip_0x(input);
            if s.len() > 8 {
                println!(
                    "{meta_prefix}{}",
                    pal.dim("[unable to decode args - complex or unsupported type]")
                );
                println!("{meta_prefix}{} {input}", pal.dim("data:"));
                data_printed = true;
            }
        }
    }

    if include_calldata && !data_printed {
        if let Some(input) = &node.input {
            println!("{meta_prefix}{} {input}", pal.dim("data:"));
        }
    }

    for child in &node.calls {
        let is_last_item = item_idx == total_items - 1;
        print_call(
            child,
            &child_prefix,
            is_last_item,
            resolver,
            contract_resolver,
            include_args,
            include_calldata,
            include_logs,
            pal,
        );
        item_idx += 1;
    }
}

/// Build the display string for a call: `address::functionName(args){value: N}`.
fn format_call_desc(
    addr: &str,
    sig: &str,
    value: Option<alloy_primitives::U256>,
    pal: Palette,
) -> String {
    let colored_addr = pal.cyan(addr);
    let value_part = value.map(|v| pal.yellow(&format!("{{value: {v}}}")));

    let sig_part = if sig.is_empty() {
        String::new()
    } else if let Some(paren) = sig.find('(') {
        format!("::{}{}", pal.bold(&sig[..paren]), &sig[paren..])
    } else {
        format!("::{sig}")
    };

    match value_part {
        Some(val_s) if sig_part.is_empty() => format!("{colored_addr}{val_s}"),
        Some(val_s) if sig.contains('(') => {
            let paren = sig.find('(').unwrap();
            format!(
                "{colored_addr}::{}{val_s}{}",
                pal.bold(&sig[..paren]),
                &sig[paren..]
            )
        }
        Some(val_s) => format!("{colored_addr}{sig_part}{val_s}"),
        None if sig_part.is_empty() => colored_addr,
        None => format!("{colored_addr}{sig_part}"),
    }
}

fn extract_selector(input: &str) -> Option<String> {
    let s = hex_utils::strip_0x(input);
    s.get(..8).map(|sel| format!("0x{sel}"))
}

fn print_arg(
    prefix: &str,
    depth: usize,
    index: usize,
    ty: &DynSolType,
    value: &DynSolValue,
    pal: Palette,
) {
    use DynSolType as T;
    use DynSolValue as V;

    let indent = "  ".repeat(depth);
    let type_str = pal.dim(&ty.to_string());

    match (ty, value) {
        (T::Tuple(inner_types), V::Tuple(inner_values)) => {
            println!("{prefix}{indent}[{index}] {type_str}");
            let len = inner_types.len().min(inner_values.len());
            for i in 0..len {
                print_arg(prefix, depth + 1, i, &inner_types[i], &inner_values[i], pal);
            }
        }
        (
            T::Array(inner_ty) | T::FixedArray(inner_ty, _),
            V::Array(inner_values) | V::FixedArray(inner_values),
        ) => {
            println!("{prefix}{indent}[{index}] {type_str}");
            for (i, v) in inner_values.iter().enumerate() {
                print_arg(prefix, depth + 1, i, inner_ty, v, pal);
            }
        }
        _ => {
            println!(
                "{prefix}{indent}[{index}] {type_str} = {}",
                abi_decoder::format_value(value)
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_selector() {
        assert_eq!(
            extract_selector("0xa9059cbb000000"),
            Some("0xa9059cbb".to_string())
        );
        assert_eq!(
            extract_selector("a9059cbb000000"),
            Some("0xa9059cbb".to_string())
        );
        assert_eq!(extract_selector("0x123"), None);
    }

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
