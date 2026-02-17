/// Parse a hex string (with or without 0x prefix) as u64.
pub fn parse_hex_u64(s: &str) -> Option<u64> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    u64::from_str_radix(s, 16).ok()
}

/// Parse a hex string (with or without 0x prefix) as u128.
pub fn parse_hex_u128(s: &str) -> Option<u128> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    u128::from_str_radix(s, 16).ok()
}

/// Parse a hex string (with or without 0x prefix) as U256.
pub fn parse_hex_u256(s: &str) -> Option<alloy_primitives::U256> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    alloy_primitives::U256::from_str_radix(s, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hex_u64() {
        assert_eq!(parse_hex_u64("0x10"), Some(16));
        assert_eq!(parse_hex_u64("10"), Some(16));
        assert_eq!(parse_hex_u64("0xff"), Some(255));
        assert_eq!(parse_hex_u64("0xffffffffffffffff"), Some(u64::MAX));
        assert_eq!(parse_hex_u64("invalid"), None);
    }

    #[test]
    fn test_parse_hex_u128() {
        assert_eq!(parse_hex_u128("0x64"), Some(100));
        assert_eq!(parse_hex_u128("64"), Some(100));
        assert_eq!(
            parse_hex_u128("0xc2c65623ae9b8000"),
            Some(14035000000000000000)
        );
        assert_eq!(parse_hex_u128("invalid"), None);
    }
}
