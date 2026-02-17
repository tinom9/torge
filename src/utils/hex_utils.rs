use alloy_primitives::U256;

/// Parse a hex string (with or without 0x prefix) as U256.
pub fn parse_hex_u256(s: &str) -> Option<U256> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    U256::from_str_radix(s, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hex_u256() {
        assert_eq!(parse_hex_u256("0x10"), Some(U256::from(16)));
        assert_eq!(parse_hex_u256("10"), Some(U256::from(16)));
        assert_eq!(parse_hex_u256("0xff"), Some(U256::from(255)));
        assert_eq!(
            parse_hex_u256("0xffffffffffffffff"),
            Some(U256::from(u64::MAX))
        );
        assert_eq!(
            parse_hex_u256("0xc2c65623ae9b8000"),
            Some(U256::from(14_035_000_000_000_000_000_u128))
        );
        assert_eq!(parse_hex_u256("invalid"), None);
    }
}
