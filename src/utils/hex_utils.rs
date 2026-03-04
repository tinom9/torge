use alloy_primitives::U256;

pub fn strip_0x(s: &str) -> &str {
    s.strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s)
}

pub fn require_0x(s: &str) -> Option<&str> {
    s.strip_prefix("0x").or_else(|| s.strip_prefix("0X"))
}

pub fn parse_hex_u256(s: &str) -> Option<U256> {
    let s = strip_0x(s);
    U256::from_str_radix(s, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_0x() {
        assert_eq!(strip_0x("0xabc"), "abc");
        assert_eq!(strip_0x("0Xabc"), "abc");
        assert_eq!(strip_0x("abc"), "abc");
        assert_eq!(strip_0x("0x"), "");
        assert_eq!(strip_0x("0X"), "");
    }

    #[test]
    fn test_require_0x() {
        assert_eq!(require_0x("0xabc"), Some("abc"));
        assert_eq!(require_0x("0Xabc"), Some("abc"));
        assert_eq!(require_0x("abc"), None);
        assert_eq!(require_0x("0x"), Some(""));
    }

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
