use crate::utils::{abi_decoder, hex_utils, selector_resolver::SelectorResolver};
use alloy_dyn_abi::DynSolType;
use serde::Deserialize;

/// A log entry (event) emitted during execution.
#[derive(Debug, Deserialize)]
pub struct Log {
    #[serde(default)]
    pub topics: Vec<String>,
    pub data: Option<String>,
}

/// Print an event log with the appropriate formatting.
pub fn print_log(log: &Log, prefix: &str, is_last: bool, resolver: &mut SelectorResolver) {
    let connector = if is_last { "└─ " } else { "├─ " };
    let resolve_events = resolver.is_enabled();

    if log.topics.is_empty() {
        let data_display = log.data.as_deref().unwrap_or("0x");
        println!("{prefix}{connector}emit <anonymous>(data: {data_display})");
        return;
    }

    let topic0 = &log.topics[0];

    let event_display = if resolve_events {
        if let Some(event_sig) = resolver.resolve_event(topic0) {
            format_event_with_signature(&event_sig, log)
        } else {
            format_event_raw(topic0, log)
        }
    } else {
        format_event_raw(topic0, log)
    };

    println!("{prefix}{connector}emit {event_display}");
}

fn format_event_with_signature(signature: &str, log: &Log) -> String {
    let name = signature.find('(').map_or(signature, |i| &signature[..i]);

    let params = decode_event_params(signature, log);
    if params.is_empty() {
        return format!("{name}()");
    }

    let param_strs: Vec<String> = params
        .iter()
        .enumerate()
        .map(|(i, value)| format!("param{i}: {value}"))
        .collect();

    format!("{name}({})", param_strs.join(", "))
}

/// Format an unresolved event with full topic0, params, and data.
fn format_event_raw(topic0: &str, log: &Log) -> String {
    let mut parts = Vec::new();

    for (i, topic) in log.topics.iter().skip(1).enumerate() {
        parts.push(format!("param{i}: {}", format_topic_value(topic)));
    }

    if let Some(data) = &log.data {
        let stripped = data.strip_prefix("0x").unwrap_or(data);
        if !stripped.is_empty() {
            parts.push(format!("data: {data}"));
        }
    }

    if parts.is_empty() {
        topic0.to_string()
    } else {
        format!("{topic0}({})", parts.join(", "))
    }
}

/// Decode event parameters from topics and data.
///
/// NOTE: Assumes indexed params (in topics) correspond to the first N params in the signature.
/// Events with non-contiguous indexed params (e.g., `event E(uint a, address indexed b, bytes c)`)
/// won't decode correctly since we can't distinguish indexed vs non-indexed from the signature alone.
fn decode_event_params(signature: &str, log: &Log) -> Vec<String> {
    let mut params = Vec::new();

    for topic in log.topics.iter().skip(1) {
        params.push(format_topic_value(topic));
    }

    if let Some(data) = &log.data {
        if let Some(decoded) = decode_event_data(signature, data, log.topics.len() - 1) {
            params.extend(decoded);
        }
    }

    params
}

fn decode_event_data(signature: &str, data: &str, indexed_count: usize) -> Option<Vec<String>> {
    let stripped = data.strip_prefix("0x").unwrap_or(data);
    if stripped.is_empty() {
        return Some(Vec::new());
    }

    let bytes = hex::decode(stripped).ok()?;

    let start = signature.find('(')?;
    let end = signature.rfind(')')?;
    let params_str = &signature[start + 1..end];

    if params_str.is_empty() {
        return Some(Vec::new());
    }

    let param_types = split_params(params_str);
    if indexed_count >= param_types.len() {
        return Some(Vec::new());
    }

    let non_indexed: Vec<&str> = param_types.iter().skip(indexed_count).copied().collect();
    if non_indexed.is_empty() {
        return Some(Vec::new());
    }

    let tuple_str = format!("({})", non_indexed.join(","));
    let param_type: DynSolType = tuple_str.parse().ok()?;

    let decoded = param_type.abi_decode_params(&bytes).ok()?;

    Some(format_decoded_values(&decoded))
}

/// Split a comma-separated parameter list respecting nested parentheses.
///
/// `"address,address,(int256,int256)"` → `["address", "address", "(int256,int256)"]`
fn split_params(s: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut depth = 0;
    let mut start = 0;

    for (i, ch) in s.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                result.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }

    if start <= s.len() {
        let tail = &s[start..];
        if !tail.is_empty() {
            result.push(tail);
        }
    }

    result
}

fn format_decoded_values(value: &alloy_dyn_abi::DynSolValue) -> Vec<String> {
    use alloy_dyn_abi::DynSolValue;

    match value {
        DynSolValue::Tuple(values) => values.iter().map(abi_decoder::format_value).collect(),
        _ => vec![abi_decoder::format_value(value)],
    }
}

/// Format a topic value (bytes32) as a readable value.
/// Tries to parse as number (must fit in u128 as a heuristic to distinguish
/// numbers from hashes), then detects addresses; otherwise shows full hex.
fn format_topic_value(topic: &str) -> String {
    let stripped = topic.strip_prefix("0x").unwrap_or(topic);

    // Heuristic: if the value fits in u128, it's likely a number.
    // Full 32-byte values (hashes, large IDs) won't fit.
    if let Some(num) = hex_utils::parse_hex_u256(topic) {
        if num <= alloy_primitives::U256::from(u128::MAX) {
            return num.to_string();
        }
    }

    // Detect address format (24 hex leading zeros + 20-byte address).
    if stripped.len() == 64 && stripped.starts_with("000000000000000000000000") {
        let addr = &stripped[24..];
        return format!("0x{addr}");
    }

    topic.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_topic_value_address() {
        let topic = "0x000000000000000000000000c02aaa39b223fe8d0a0e5c4f27ead9083c756cc2";
        assert_eq!(
            format_topic_value(topic),
            "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2"
        );
    }

    #[test]
    fn test_format_topic_value_number() {
        let topic = "0x0000000000000000000000000000000000000000000000000000000000000064";
        assert_eq!(format_topic_value(topic), "100");
    }

    #[test]
    fn test_format_topic_value_large_number() {
        let topic = "0x000000000000000000000000000000000000000000000000c2c65623ae9b8000";
        assert_eq!(format_topic_value(topic), "14035000000000000000");
    }

    #[test]
    fn test_format_topic_value_hash() {
        let topic = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";
        assert_eq!(format_topic_value(topic), topic);
    }

    #[test]
    fn test_format_event_raw() {
        let log = Log {
            topics: vec![
                "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef".into(),
                "0x000000000000000000000000e3100bb16871d9f53a5bc8a659803811a4d08e59".into(),
            ],
            data: Some("0x000000000000000000000000000000000000000000000000c2c65623ae9b8000".into()),
        };
        let result = format_event_raw(&log.topics[0], &log);
        assert!(result
            .starts_with("0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"));
        assert!(result.contains("param0: 0xe3100bb16871d9f53a5bc8a659803811a4d08e59"));
        assert!(result
            .contains("data: 0x000000000000000000000000000000000000000000000000c2c65623ae9b8000"));
    }

    #[test]
    fn test_format_event_with_signature() {
        let log = Log {
            topics: vec![
                "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef".into(),
                "0x000000000000000000000000e3100bb16871d9f53a5bc8a659803811a4d08e59".into(),
                "0x00000000000000000000000066a9893cc07d91d95644aedd05d03f95e1dba8af".into(),
            ],
            data: Some("0x000000000000000000000000000000000000000000000000c2c65623ae9b8000".into()),
        };
        let result = format_event_with_signature("Transfer(address,address,uint256)", &log);
        assert_eq!(
            result,
            "Transfer(param0: 0xe3100bb16871d9f53a5bc8a659803811a4d08e59, param1: 0x66a9893cc07d91d95644aedd05d03f95e1dba8af, param2: 14035000000000000000)"
        );
    }

    #[test]
    fn test_split_params_simple() {
        assert_eq!(
            split_params("address,address,uint256"),
            vec!["address", "address", "uint256"]
        );
    }

    #[test]
    fn test_split_params_tuple() {
        assert_eq!(
            split_params("address,(int256,int256)"),
            vec!["address", "(int256,int256)"]
        );
    }

    #[test]
    fn test_split_params_nested() {
        assert_eq!(
            split_params("address,((uint160,int24),uint256)"),
            vec!["address", "((uint160,int24),uint256)"]
        );
    }

    #[test]
    fn test_split_params_empty() {
        let result: Vec<&str> = split_params("");
        assert!(result.is_empty());
    }

    #[test]
    fn test_split_params_single() {
        assert_eq!(split_params("uint256"), vec!["uint256"]);
    }

    #[test]
    fn test_format_event_with_tuple_param() {
        // Simulates: event Swap(address indexed sender, address indexed recipient, (int256 amount0, int256 amount1))
        // topic0 = keccak256, topic1 = sender, topic2 = recipient, data = tuple(int256, int256)
        let log = Log {
            topics: vec![
                "0xc42079f94a6350d7e6235f29174924f928cc2ac818eb64fed8004e115fbcca67".into(),
                "0x000000000000000000000000e592427a0aece92de3edee1f18e0157c05861564".into(),
                "0x000000000000000000000000e592427a0aece92de3edee1f18e0157c05861564".into(),
            ],
            // ABI-encoded (int256(-1000), int256(2000))
            data: Some(
                "0xfffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffc18\
                         00000000000000000000000000000000000000000000000000000000000007d0"
                    .into(),
            ),
        };
        let result = format_event_with_signature("Swap(address,address,(int256,int256))", &log);
        assert!(result.starts_with("Swap("));
        // 3 params total: 2 indexed topics + 1 decoded tuple
        assert!(result.contains("param0:"));
        assert!(result.contains("param1:"));
        assert!(result.contains("param2:"));
        // The tuple is a single param, so no param3
        assert!(!result.contains("param3:"));
    }

    #[test]
    fn test_format_event_with_signature_no_params() {
        let log = Log {
            topics: vec![
                "0xe1fffcc4923d04b559f4d29a8bfc6cda04eb5b0d3c460751c2402c5c5cc9109c".into(),
            ],
            data: None,
        };
        let result = format_event_with_signature("SomeEvent()", &log);
        assert_eq!(result, "SomeEvent()");
    }
}
