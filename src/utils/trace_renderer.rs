use crate::utils::{
    abi_decoder,
    color::Palette,
    contract_resolver::ContractResolver,
    event_formatter::{self, print_log},
    hex_utils, precompiles,
    selector_resolver::SelectorResolver,
};
use alloy_dyn_abi::{DynSolType, DynSolValue};
use serde::Deserialize;

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
    pub logs: Vec<event_formatter::Log>,
    #[serde(default)]
    pub calls: Vec<CallTrace>,
}

pub(crate) fn print_trace(
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

pub(crate) fn extract_selector(input: &str) -> Option<String> {
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
    fn test_extract_selector_uppercase_prefix() {
        assert_eq!(
            extract_selector("0Xa9059cbb000000"),
            Some("0xa9059cbb".to_string())
        );
    }
}
