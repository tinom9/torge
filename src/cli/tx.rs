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
use serde_json::json;
use std::time::Duration;
use thiserror::Error;

#[derive(Parser, Debug)]
pub struct TxArgs {
    /// Transaction hash to trace (0x-prefixed).
    pub tx_hash: String,

    /// Ethereum JSON-RPC URL or Foundry alias (e.g. http://localhost:8545 or ethereum).
    ///
    /// If a URL is provided (starts with http:// or https://), it will be used directly.
    /// Otherwise, the tool will look for a matching alias in foundry.toml's [rpc_endpoints].
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
}

#[derive(Debug, Error)]
pub enum TxError {
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
struct CallTrace {
    #[serde(rename = "type")]
    call_type: Option<String>,
    to: Option<String>,
    value: Option<String>,
    gas_used: Option<String>,
    input: Option<String>,
    error: Option<String>,
    #[serde(default)]
    logs: Vec<Log>,
    #[serde(default)]
    calls: Vec<CallTrace>,
}

pub fn run(args: TxArgs) -> Result<(), TxError> {
    if args.include_args && !args.resolve_selectors {
        return Err(TxError::IncludeArgsRequiresResolveSelectors);
    }

    let rpc_url = match args.rpc_url {
        Some(url_or_alias) => rpc_url::resolve(&url_or_alias)?,
        None => std::env::var("RPC_URL")
            .or_else(|_| std::env::var("ETH_RPC_URL"))
            .map_err(|_| TxError::MissingRpcUrl)?,
    };

    let client = Client::builder()
        .timeout(Duration::from_secs(60))
        .no_proxy()
        .build()?;

    let trace = debug_trace_transaction(&client, &rpc_url, &args.tx_hash, args.include_logs)?;

    let mut resolver = SelectorResolver::new(&client, args.resolve_selectors);
    print_trace(
        &trace,
        &mut resolver,
        args.include_args,
        args.include_calldata,
        args.include_logs,
    );

    Ok(())
}

fn debug_trace_transaction(
    client: &Client,
    rpc_url: &str,
    tx_hash: &str,
    include_logs: bool,
) -> Result<CallTrace, TxError> {
    let payload = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "debug_traceTransaction",
        "params": [
            tx_hash,
            {
                "tracer": "callTracer",
                "tracerConfig": {
                    "onlyTopCall": false,
                    "withLog": include_logs,
                }
            }
        ]
    });

    let resp = client
        .post(rpc_url)
        .json(&payload)
        .send()?
        .error_for_status()?;

    let rpc_resp: RpcResponse<CallTrace> = resp.json()?;

    if let Some(err) = rpc_resp.error {
        return Err(TxError::Rpc(err.message, err.code));
    }

    rpc_resp
        .result
        .ok_or_else(|| TxError::Decode("missing result field in RPC response".into()))
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
        .and_then(hex_utils::parse_hex_u64)
        .unwrap_or(0);

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
    let indent = "  ".repeat(depth);
    let type_str = abi_decoder::format_param_type(ty);

    use DynSolType as T;
    use DynSolValue as V;

    match (ty, value) {
        (T::Tuple(inner_types), V::Tuple(inner_values)) => {
            println!("{prefix}{indent}[{index}] {type_str}");
            let len = inner_types.len().min(inner_values.len());
            for i in 0..len {
                print_arg(prefix, depth + 1, i, &inner_types[i], &inner_values[i]);
            }
        }
        (T::Array(inner_ty), V::Array(inner_values))
        | (T::Array(inner_ty), V::FixedArray(inner_values))
        | (T::FixedArray(inner_ty, _), V::Array(inner_values))
        | (T::FixedArray(inner_ty, _), V::FixedArray(inner_values)) => {
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
}
