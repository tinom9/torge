use alloy_dyn_abi::{DynSolType, DynSolValue};

/// Decode precompile arguments (no 4-byte selector prefix).
///
/// Precompiles receive raw input data without a function selector prefix.
pub fn decode_precompile_args(
    signature: &str,
    input: &str,
) -> Option<Vec<(DynSolType, DynSolValue)>> {
    let hex = input.strip_prefix("0x").unwrap_or(input);
    decode_args(signature, hex)
}

/// Decode ABI tokens given a text signature and full calldata hex, paired with
/// their corresponding dynamic ABI types.
pub fn decode_function_args(
    signature: &str,
    input: &str,
) -> Option<Vec<(DynSolType, DynSolValue)>> {
    let hex = input.strip_prefix("0x").unwrap_or(input);
    hex.get(8..)
        .and_then(|args_hex| decode_args(signature, args_hex))
}

/// Shared ABI decoding: parse signature, decode bytes, split into typed pairs.
fn decode_args(signature: &str, hex_data: &str) -> Option<Vec<(DynSolType, DynSolValue)>> {
    let bytes = hex::decode(hex_data).ok()?;

    let start = signature.find('(')?;
    let params_str = signature.get(start..)?;
    let param_type: DynSolType = params_str.parse().ok()?;

    if matches!(param_type, DynSolType::Tuple(ref v) if v.is_empty()) && bytes.is_empty() {
        return Some(Vec::new());
    }

    let decoded = param_type.abi_decode_params(&bytes).ok()?;
    Some(split_types_and_values(param_type, decoded))
}

/// Check if a signature can decode the given calldata.
pub fn can_decode(signature: &str, calldata: &str) -> bool {
    let s = calldata.strip_prefix("0x").unwrap_or(calldata);
    if s.len() < 8 {
        return false;
    }

    let args_hex = &s[8..];
    let Ok(args_bytes) = hex::decode(args_hex) else {
        return false;
    };

    let Some(start) = signature.find('(') else {
        return false;
    };
    let Some(params_str) = signature.get(start..) else {
        return false;
    };
    let Ok(param_type) = params_str.parse::<DynSolType>() else {
        return false;
    };

    if matches!(param_type, DynSolType::Tuple(ref v) if v.is_empty()) && args_bytes.is_empty() {
        return true;
    }

    param_type.abi_decode_params(&args_bytes).is_ok()
}

/// Split a decoded `DynSolValue` into (type, value) pairs for each parameter.
fn split_types_and_values(
    param_type: DynSolType,
    value: DynSolValue,
) -> Vec<(DynSolType, DynSolValue)> {
    match (param_type, value) {
        (DynSolType::Tuple(types), DynSolValue::Tuple(values)) => {
            let n = types.len().min(values.len());
            (0..n)
                .map(|i| (types[i].clone(), values[i].clone()))
                .collect()
        }
        (ty, val) => vec![(ty, val)],
    }
}

pub fn format_param_type(kind: &DynSolType) -> String {
    match kind {
        DynSolType::Address => "address".to_owned(),
        DynSolType::Bool => "bool".to_owned(),
        DynSolType::Uint(n) => format!("uint{n}"),
        DynSolType::Int(n) => format!("int{n}"),
        DynSolType::Bytes => "bytes".to_owned(),
        DynSolType::FixedBytes(n) => format!("bytes{n}"),
        DynSolType::String => "string".to_owned(),
        DynSolType::Function => "function".to_owned(),
        DynSolType::Array(inner) => format!("{}[]", format_param_type(inner)),
        DynSolType::FixedArray(inner, n) => format!("{}[{n}]", format_param_type(inner)),
        DynSolType::Tuple(inner) => {
            let parts: Vec<_> = inner.iter().map(format_param_type).collect();
            format!("({})", parts.join(","))
        }
        #[allow(unreachable_patterns)]
        _ => kind.to_string(),
    }
}

/// Decode a revert reason from raw output bytes (0x-prefixed hex).
///
/// Recognises:
/// - `Error(string)` — selector `0x08c379a0`
/// - `Panic(uint256)` — selector `0x4e487b71`
pub fn decode_revert_reason(output: &str) -> Option<String> {
    let hex = output.strip_prefix("0x").unwrap_or(output);
    if hex.len() < 8 {
        return None;
    }

    let selector = &hex[..8];
    let data = hex::decode(&hex[8..]).ok()?;

    match selector {
        // Error(string)
        "08c379a0" => {
            let ty: DynSolType = "(string)".parse().ok()?;
            let decoded = ty.abi_decode_params(&data).ok()?;
            if let DynSolValue::Tuple(vals) = decoded {
                vals.first().map(format_value)
            } else {
                Some(format_value(&decoded))
            }
        }
        // Panic(uint256)
        "4e487b71" => {
            let ty: DynSolType = "(uint256)".parse().ok()?;
            let decoded = ty.abi_decode_params(&data).ok()?;
            let code_val = if let DynSolValue::Tuple(vals) = &decoded {
                vals.first().map(format_value)
            } else {
                Some(format_value(&decoded))
            };
            let code_str = code_val.unwrap_or_default();
            let desc = panic_description(&code_str);
            if desc.is_empty() {
                Some(format!("Panic({code_str})"))
            } else {
                Some(format!("Panic({code_str}: {desc})"))
            }
        }
        _ => None,
    }
}

/// Map a Solidity panic code to a human-readable description.
fn panic_description(code: &str) -> &'static str {
    match code {
        "0" => "generic/compiler-inserted",
        "1" => "assertion failed",
        "17" => "arithmetic overflow/underflow",
        "18" => "division or modulo by zero",
        "33" => "invalid enum conversion",
        "34" => "invalid storage access",
        "49" => "pop on empty array",
        "50" => "out-of-bounds array access",
        "65" => "out of memory",
        "81" => "call to zero-initialized function",
        _ => "",
    }
}

pub fn format_value(value: &DynSolValue) -> String {
    match value {
        DynSolValue::Address(a) => format!("{a:#x}"),
        DynSolValue::Uint(u, _) => format!("{u}"),
        DynSolValue::Int(i, _) => format!("{i}"),
        DynSolValue::Bool(b) => b.to_string(),
        DynSolValue::String(s) => s.clone(),
        DynSolValue::Bytes(b) => format!("0x{}", hex::encode(b)),
        DynSolValue::FixedBytes(b, _) => format!("0x{}", hex::encode(b)),
        DynSolValue::Array(inner) | DynSolValue::FixedArray(inner) => {
            let parts: Vec<_> = inner.iter().map(format_value).collect();
            format!("[{}]", parts.join(", "))
        }
        DynSolValue::Tuple(inner) => {
            let parts: Vec<_> = inner.iter().map(format_value).collect();
            format!("({})", parts.join(", "))
        }
        DynSolValue::Function(_) => format!("{value:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_can_decode_valid_transfer() {
        let sig = "transfer(address,uint256)";
        // transfer(0xdead...beef, 1000)
        let calldata = "0xa9059cbb000000000000000000000000deadbeefdeadbeefdeadbeefdeadbeefdeadbeef00000000000000000000000000000000000000000000000000000000000003e8";
        assert!(can_decode(sig, calldata));
    }

    #[test]
    fn test_can_decode_invalid_calldata() {
        let sig = "transfer(address,uint256)";
        let calldata = "0xa9059cbb00"; // truncated
        assert!(!can_decode(sig, calldata));
    }

    #[test]
    fn test_can_decode_empty_args() {
        let sig = "pause()";
        let calldata = "0x8456cb59";
        assert!(can_decode(sig, calldata));
    }

    #[test]
    fn test_can_decode_short_calldata() {
        let sig = "transfer(address,uint256)";
        let calldata = "0x1234"; // less than 4 bytes
        assert!(!can_decode(sig, calldata));
    }

    #[test]
    fn test_decode_function_args_transfer() {
        let sig = "transfer(address,uint256)";
        let calldata = "0xa9059cbb000000000000000000000000deadbeefdeadbeefdeadbeefdeadbeefdeadbeef00000000000000000000000000000000000000000000000000000000000003e8";

        let result = decode_function_args(sig, calldata);
        assert!(result.is_some());

        let args = result.unwrap();
        assert_eq!(args.len(), 2);
    }

    #[test]
    fn test_decode_function_args_empty_params() {
        let sig = "pause()";
        let calldata = "0x8456cb59";

        let result = decode_function_args(sig, calldata);
        assert!(result.is_some());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_format_param_type_primitives() {
        assert_eq!(format_param_type(&DynSolType::Address), "address");
        assert_eq!(format_param_type(&DynSolType::Bool), "bool");
        assert_eq!(format_param_type(&DynSolType::Uint(256)), "uint256");
        assert_eq!(format_param_type(&DynSolType::Int(128)), "int128");
        assert_eq!(format_param_type(&DynSolType::Bytes), "bytes");
        assert_eq!(format_param_type(&DynSolType::FixedBytes(32)), "bytes32");
        assert_eq!(format_param_type(&DynSolType::String), "string");
    }

    #[test]
    fn test_format_param_type_arrays() {
        assert_eq!(
            format_param_type(&DynSolType::Array(Box::new(DynSolType::Address))),
            "address[]"
        );
        assert_eq!(
            format_param_type(&DynSolType::FixedArray(Box::new(DynSolType::Uint(256)), 3)),
            "uint256[3]"
        );
    }

    #[test]
    fn test_format_param_type_tuple() {
        let tuple = DynSolType::Tuple(vec![DynSolType::Address, DynSolType::Uint(256)]);
        assert_eq!(format_param_type(&tuple), "(address,uint256)");
    }

    #[test]
    fn test_format_value_bool() {
        assert_eq!(format_value(&DynSolValue::Bool(true)), "true");
        assert_eq!(format_value(&DynSolValue::Bool(false)), "false");
    }

    #[test]
    fn test_format_value_string() {
        assert_eq!(
            format_value(&DynSolValue::String("hello".to_owned())),
            "hello"
        );
    }

    #[test]
    fn test_format_value_bytes() {
        assert_eq!(
            format_value(&DynSolValue::Bytes(vec![0xde, 0xad, 0xbe, 0xef])),
            "0xdeadbeef"
        );
    }

    #[test]
    fn test_decode_revert_error_string() {
        // Error("ERC20: transfer amount exceeds balance")
        // selector 08c379a0 + abi-encoded string
        let output = "0x08c379a0\
            0000000000000000000000000000000000000000000000000000000000000020\
            0000000000000000000000000000000000000000000000000000000000000026\
            45524332303a207472616e7366657220616d6f756e7420657863656564732062\
            616c616e63650000000000000000000000000000000000000000000000000000";
        let reason = decode_revert_reason(output);
        assert!(reason.is_some());
        assert_eq!(reason.unwrap(), "ERC20: transfer amount exceeds balance");
    }

    #[test]
    fn test_decode_revert_panic() {
        // Panic(0x11) — arithmetic overflow
        let output = "0x4e487b710000000000000000000000000000000000000000000000000000000000000011";
        let reason = decode_revert_reason(output);
        assert!(reason.is_some());
        assert_eq!(reason.unwrap(), "Panic(17: arithmetic overflow/underflow)");
    }

    #[test]
    fn test_decode_revert_unknown_selector() {
        let output = "0xdeadbeef0000000000000000000000000000000000000000000000000000000000000001";
        assert!(decode_revert_reason(output).is_none());
    }

    #[test]
    fn test_decode_revert_too_short() {
        assert!(decode_revert_reason("0x1234").is_none());
    }
}
