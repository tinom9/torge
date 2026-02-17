use crate::utils::{
    abi_decoder,
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
pub struct RpcResponse<T> {
    pub result: Option<T>,
    pub error: Option<RpcError>,
}

#[derive(Debug, Deserialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
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
pub fn resolve_rpc_url(url_or_alias: Option<String>) -> Result<String, TraceError> {
    match url_or_alias {
        Some(val) => Ok(rpc_url::resolve(&val)?),
        None => std::env::var("RPC_URL")
            .or_else(|_| std::env::var("ETH_RPC_URL"))
            .map_err(|_| TraceError::MissingRpcUrl),
    }
}

/// Validate that a string is a 0x-prefixed Ethereum address (40 hex chars).
pub fn validate_address(addr: &str, field: &str) -> Result<(), TraceError> {
    let hex = addr
        .strip_prefix("0x")
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

/// Validate that a string is 0x-prefixed hex data.
pub fn validate_hex(data: &str, field: &str) -> Result<(), TraceError> {
    let hex = data
        .strip_prefix("0x")
        .ok_or_else(|| TraceError::InvalidInput(format!("{field}: missing 0x prefix")))?;
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(TraceError::InvalidInput(format!(
            "{field}: invalid hex characters"
        )));
    }
    Ok(())
}

/// Send an RPC payload, parse the trace response, and print it.
pub fn execute_and_print(payload: &serde_json::Value, opts: TraceOpts) -> Result<(), TraceError> {
    if opts.include_args && !opts.resolve_selectors {
        return Err(TraceError::IncludeArgsRequiresResolveSelectors);
    }

    let rpc_url = resolve_rpc_url(opts.rpc_url)?;
    let client = create_client(opts.no_proxy)?;

    let resp = client.post(&rpc_url).json(payload).send()?;

    let call_trace = parse_rpc_response(resp)?;

    let mut resolver = SelectorResolver::new(client, opts.resolve_selectors);
    print_trace(
        &call_trace,
        &mut resolver,
        opts.include_args,
        opts.include_calldata,
        opts.include_logs,
    );

    Ok(())
}

/// Parse the RPC response, returning the result or an error.
fn parse_rpc_response(resp: reqwest::blocking::Response) -> Result<CallTrace, TraceError> {
    let rpc_resp: RpcResponse<CallTrace> = resp.json()?;

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
    include_args: bool,
    include_calldata: bool,
    include_logs: bool,
) {
    println!("Traces:");
    print_call(
        root,
        "",
        true,
        resolver,
        include_args,
        include_calldata,
        include_logs,
    );
}

#[allow(clippy::too_many_lines, clippy::fn_params_excessive_bools)]
fn print_call(
    node: &CallTrace,
    prefix: &str,
    is_last: bool,
    resolver: &mut SelectorResolver,
    include_args: bool,
    include_calldata: bool,
    include_logs: bool,
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

    let (display_addr, sig) = if let Some((_name, signature)) = precompile {
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

        (to, sig)
    };

    let call_desc = if let Some(val) = value {
        if sig.is_empty() {
            format!("{display_addr}{{value: {val}}}")
        } else if let Some(paren_pos) = sig.find('(') {
            format!(
                "{display_addr}::{}{{value: {}}}{}",
                &sig[..paren_pos],
                val,
                &sig[paren_pos..]
            )
        } else {
            format!("{display_addr}::{sig}{{value: {val}}}")
        }
    } else if sig.is_empty() {
        display_addr.to_string()
    } else {
        format!("{display_addr}::{sig}")
    };

    let call_type = node.call_type.as_deref().unwrap_or("").to_uppercase();

    let call_type_suffix = match call_type.as_str() {
        "DELEGATECALL" => " [delegatecall]",
        "STATICCALL" => " [staticcall]",
        "CALLCODE" => " [callcode]",
        "CREATE" => " [create]",
        "CREATE2" => " [create2]",
        _ if !call_type.is_empty() => " [call]",
        _ => "",
    };

    let connector = if prefix.is_empty() {
        ""
    } else if is_last {
        "└─ "
    } else {
        "├─ "
    };

    println!("{prefix}{connector}[{gas_used}] {call_desc}{call_type_suffix}");

    if let Some(err) = &node.error {
        println!("{prefix}    ↳ error: {err}");
    }

    let child_prefix = format!("{}{}", prefix, if is_last { "    " } else { "│   " });
    let meta_prefix = &child_prefix;

    let logs_count = if include_logs { node.logs.len() } else { 0 };
    let total_items = logs_count + node.calls.len();
    let mut item_idx = 0;

    if include_logs {
        for log in &node.logs {
            let is_last_item = item_idx == total_items - 1;
            print_log(log, &child_prefix, is_last_item, resolver);
            item_idx += 1;
        }
    }

    let mut data_printed = false;

    if let Some(args) = decoded {
        if !args.is_empty() {
            println!("{meta_prefix}args:");

            if args.len() == 1 {
                if let (DynSolType::Tuple(inner_types), DynSolValue::Tuple(inner_values)) = &args[0]
                {
                    let len = inner_types.len().min(inner_values.len());
                    for i in 0..len {
                        print_arg(meta_prefix, 0, i, &inner_types[i], &inner_values[i]);
                    }
                } else {
                    for (i, (ty, value)) in args.iter().enumerate() {
                        print_arg(meta_prefix, 0, i, ty, value);
                    }
                }
            } else {
                for (i, (ty, value)) in args.iter().enumerate() {
                    print_arg(meta_prefix, 0, i, ty, value);
                }
            }
        }
    } else if decode_attempted {
        if let Some(input) = &node.input {
            let s = input.strip_prefix("0x").unwrap_or(input);
            if s.len() > 8 {
                println!("{meta_prefix}[unable to decode args - complex or unsupported type]");
                println!("{meta_prefix}data: {input}");
                data_printed = true;
            }
        }
    }

    if include_calldata && !data_printed {
        if let Some(input) = &node.input {
            println!("{meta_prefix}data: {input}");
        }
    }

    for child in &node.calls {
        let is_last_item = item_idx == total_items - 1;
        print_call(
            child,
            &child_prefix,
            is_last_item,
            resolver,
            include_args,
            include_calldata,
            include_logs,
        );
        item_idx += 1;
    }
}

fn extract_selector(input: &str) -> Option<String> {
    let s = input.strip_prefix("0x").unwrap_or(input);
    s.get(..8).map(|sel| format!("0x{sel}"))
}

fn print_arg(prefix: &str, depth: usize, index: usize, ty: &DynSolType, value: &DynSolValue) {
    use DynSolType as T;
    use DynSolValue as V;

    let indent = "  ".repeat(depth);
    let type_str = abi_decoder::format_param_type(ty);

    match (ty, value) {
        (T::Tuple(inner_types), V::Tuple(inner_values)) => {
            println!("{prefix}{indent}[{index}] {type_str}");
            let len = inner_types.len().min(inner_values.len());
            for i in 0..len {
                print_arg(prefix, depth + 1, i, &inner_types[i], &inner_values[i]);
            }
        }
        (
            T::Array(inner_ty) | T::FixedArray(inner_ty, _),
            V::Array(inner_values) | V::FixedArray(inner_values),
        ) => {
            println!("{prefix}{indent}[{index}] {type_str}");
            for (i, v) in inner_values.iter().enumerate() {
                print_arg(prefix, depth + 1, i, inner_ty, v);
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
    }
}
