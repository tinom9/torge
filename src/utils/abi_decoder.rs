use super::hex_utils;
use alloy_dyn_abi::{DynSolType, DynSolValue, JsonAbiExt, Specifier};
use alloy_json_abi::Function;

/// Decode ABI-encoded function arguments (strips 4-byte selector).
pub fn decode_function_args(
    signature: &str,
    input: &str,
) -> Option<Vec<(DynSolType, DynSolValue)>> {
    let bytes = decode_hex(input)?;
    if bytes.len() < 4 {
        return None;
    }
    decode_with_function(signature, &bytes[4..])
}

/// Decode precompile arguments (no 4-byte selector prefix).
pub fn decode_precompile_args(
    signature: &str,
    input: &str,
) -> Option<Vec<(DynSolType, DynSolValue)>> {
    decode_with_function(signature, &decode_hex(input)?)
}

/// Check if a signature can decode the given calldata.
pub fn can_decode(signature: &str, calldata: &str) -> bool {
    decode_function_args(signature, calldata).is_some()
}

/// Decode a revert reason from raw output bytes (0x-prefixed hex).
///
/// Recognises `Error(string)` (0x08c379a0) and `Panic(uint256)` (0x4e487b71).
/// Returns `None` for unknown selectors.
pub fn decode_revert_reason(output: &str) -> Option<String> {
    let hex = hex_utils::strip_0x(output);
    if hex.len() < 8 {
        return None;
    }

    let data = hex::decode(&hex[8..]).ok()?;

    match &hex[..8] {
        "08c379a0" => {
            let func = Function::parse("Error(string)").ok()?;
            let values = func.abi_decode_input(&data).ok()?;
            values.first().map(format_value)
        }
        "4e487b71" => {
            let func = Function::parse("Panic(uint256)").ok()?;
            let values = func.abi_decode_input(&data).ok()?;
            let code_str = format_value(values.first()?);
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

fn decode_hex(input: &str) -> Option<Vec<u8>> {
    hex::decode(hex_utils::strip_0x(input)).ok()
}

fn decode_with_function(signature: &str, data: &[u8]) -> Option<Vec<(DynSolType, DynSolValue)>> {
    let func: Function = Function::parse(signature).ok()?;
    let values = func.abi_decode_input(data).ok()?;
    func.inputs
        .iter()
        .zip(values)
        .map(|(p, v)| p.resolve().ok().map(|t| (t, v)))
        .collect::<Option<Vec<_>>>()
}

/// Get the description of a Solidity panic code.
/// <https://docs.soliditylang.org/en/v0.8.34/control-structures.html#panic-via-assert-and-error-via-require>
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_can_decode_valid_transfer() {
        let sig = "transfer(address,uint256)";
        let calldata = "0xa9059cbb\
            000000000000000000000000deadbeefdeadbeefdeadbeefdeadbeefdeadbeef\
            00000000000000000000000000000000000000000000000000000000000003e8";
        assert!(can_decode(sig, calldata));
    }

    #[test]
    fn test_can_decode_wrong_signature() {
        let sig = "transfer(address,uint256)";
        let calldata = "0xa9059cbb00"; // truncated — signature doesn't match data
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
        let calldata = "0xa9059cbb\
            000000000000000000000000deadbeefdeadbeefdeadbeefdeadbeefdeadbeef\
            00000000000000000000000000000000000000000000000000000000000003e8";
        let args = decode_function_args(sig, calldata).unwrap();
        assert_eq!(args.len(), 2);
        assert!(matches!(args[0].0, DynSolType::Address));
        assert!(matches!(args[1].0, DynSolType::Uint(256)));
    }

    #[test]
    fn test_decode_function_args_empty_params() {
        let sig = "pause()";
        let calldata = "0x8456cb59";
        let result = decode_function_args(sig, calldata).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_decode_returns_none_on_mismatch() {
        let sig = "registerPool(uint8,address[],address[])";
        let calldata = "0x6634b753\
            0000000000000000000000000000000000000000000000000000000000000002";
        assert!(decode_function_args(sig, calldata).is_none());
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
        let output = "0x08c379a0\
            0000000000000000000000000000000000000000000000000000000000000020\
            0000000000000000000000000000000000000000000000000000000000000026\
            45524332303a207472616e7366657220616d6f756e7420657863656564732062\
            616c616e63650000000000000000000000000000000000000000000000000000";
        assert_eq!(
            decode_revert_reason(output).unwrap(),
            "ERC20: transfer amount exceeds balance"
        );
    }

    #[test]
    fn test_decode_revert_panic() {
        let output = "0x4e487b710000000000000000000000000000000000000000000000000000000000000011";
        assert_eq!(
            decode_revert_reason(output).unwrap(),
            "Panic(17: arithmetic overflow/underflow)"
        );
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
