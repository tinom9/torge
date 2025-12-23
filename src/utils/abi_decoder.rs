use alloy_dyn_abi::{DynSolType, DynSolValue};

/// Decode precompile arguments (no 4-byte selector prefix).
///
/// Precompiles receive raw input data without a function selector prefix.
pub fn decode_precompile_args(
    signature: &str,
    input: &str,
) -> Option<Vec<(DynSolType, DynSolValue)>> {
    let s = input.strip_prefix("0x").unwrap_or(input);
    let args_bytes = hex::decode(s).ok()?;

    let start = signature.find('(')?;
    let params_str = signature.get(start..)?;
    let param_type: DynSolType = params_str.parse().ok()?;

    if matches!(param_type, DynSolType::Tuple(ref v) if v.is_empty()) && args_bytes.is_empty() {
        return Some(Vec::new());
    }

    let decoded = param_type.abi_decode_params(&args_bytes).ok()?;
    Some(split_types_and_values(param_type, decoded))
}

/// Decode ABI tokens given a text signature and full calldata hex, paired with
/// their corresponding dynamic ABI types.
pub fn decode_function_args(
    signature: &str,
    input: &str,
) -> Option<Vec<(DynSolType, DynSolValue)>> {
    let s = input.strip_prefix("0x").unwrap_or(input);
    if s.len() < 8 {
        return None;
    }

    let args_hex = &s[8..];
    let args_bytes = hex::decode(args_hex).ok()?;

    let start = signature.find('(')?;
    let params_str = signature.get(start..)?;
    let param_type: DynSolType = params_str.parse().ok()?;

    if matches!(param_type, DynSolType::Tuple(ref v) if v.is_empty()) && args_bytes.is_empty() {
        return Some(Vec::new());
    }

    let decoded = param_type.abi_decode_params(&args_bytes).ok()?;
    Some(split_types_and_values(param_type, decoded))
}

/// Check if a signature can decode the given calldata.
pub fn can_decode(signature: &str, calldata: &str) -> bool {
    let s = calldata.strip_prefix("0x").unwrap_or(calldata);
    if s.len() < 8 {
        return false;
    }

    let args_hex = &s[8..];
    let args_bytes = match hex::decode(args_hex) {
        Ok(b) => b,
        Err(_) => return false,
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

/// Split a decoded DynSolValue into (type, value) pairs for each parameter.
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
    use DynSolType::*;

    match kind {
        Address => "address".to_owned(),
        Bool => "bool".to_owned(),
        Uint(n) => format!("uint{n}"),
        Int(n) => format!("int{n}"),
        Bytes => "bytes".to_owned(),
        FixedBytes(n) => format!("bytes{n}"),
        String => "string".to_owned(),
        Function => "function".to_owned(),
        Array(inner) => format!("{}[]", format_param_type(inner)),
        FixedArray(inner, n) => format!("{}[{n}]", format_param_type(inner)),
        Tuple(inner) => {
            let inner = inner
                .iter()
                .map(format_param_type)
                .collect::<Vec<_>>()
                .join(",");
            format!("({inner})")
        }
        #[allow(unreachable_patterns)]
        _ => kind.to_string(),
    }
}

pub fn format_value(value: &DynSolValue) -> String {
    use DynSolValue::*;

    match value {
        Address(a) => format!("{a:#x}"),
        Uint(u, _) => format!("{u}"),
        Int(i, _) => format!("{i}"),
        Bool(b) => b.to_string(),
        String(s) => s.to_string(),
        Bytes(b) => format!("0x{}", hex::encode(b)),
        FixedBytes(b, _) => format!("0x{}", hex::encode(b)),
        Array(inner) | FixedArray(inner) => {
            let inner = inner
                .iter()
                .map(format_value)
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{inner}]")
        }
        Tuple(inner) => {
            let inner = inner
                .iter()
                .map(format_value)
                .collect::<Vec<_>>()
                .join(", ");
            format!("({inner})")
        }
        _ => format!("{value:?}"),
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
}
