use super::hex_utils;
use alloy_primitives::U256;

/// Parse a value string into a `0x`-prefixed hex wei amount.
///
/// Supported formats:
/// - Hex: `0x1234`, `0xde0b6b3a7640000`
/// - Decimal: `1000000`, `0`
/// - Units: `1gwei`, `100gwei`, `1ether`, `4.5ether`, `0.01ether`
pub fn parse_value(s: &str) -> Result<String, String> {
    let s = s.trim();

    if let Some(hex) = hex_utils::require_0x(s) {
        let v = U256::from_str_radix(hex, 16).map_err(|_| format!("invalid hex value: {s}"))?;
        return Ok(format!("{v:#x}"));
    }

    if let Some(num_str) = s.strip_suffix("ether") {
        let wei = to_wei(num_str, 18)?;
        return Ok(format!("{wei:#x}"));
    }

    if let Some(num_str) = s.strip_suffix("gwei") {
        let wei = to_wei(num_str, 9)?;
        return Ok(format!("{wei:#x}"));
    }

    if let Some(num_str) = s.strip_suffix("wei") {
        let v = U256::from_str_radix(num_str, 10).map_err(|_| format!("invalid value: {s}"))?;
        return Ok(format!("{v:#x}"));
    }

    let v = U256::from_str_radix(s, 10).map_err(|_| format!("invalid value: {s}"))?;
    Ok(format!("{v:#x}"))
}

/// Convert a possibly-decimal number string to wei given the unit's decimal count.
fn to_wei(num_str: &str, decimals: usize) -> Result<U256, String> {
    let (combined, frac_len) = match num_str.split_once('.') {
        Some((int_part, frac)) => {
            if frac.len() > decimals {
                return Err(format!("too many decimal places: {num_str}"));
            }
            (format!("{int_part}{frac}"), frac.len())
        }
        None => (num_str.to_string(), 0),
    };

    let val =
        U256::from_str_radix(&combined, 10).map_err(|_| format!("invalid number: {num_str}"))?;
    Ok(val * U256::from(10u64).pow(U256::from(decimals - frac_len)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_value_hex() {
        assert_eq!(
            parse_value("0xde0b6b3a7640000").unwrap(),
            "0xde0b6b3a7640000"
        );
        assert_eq!(parse_value("0x0").unwrap(), "0x0");
    }

    #[test]
    fn test_parse_value_decimal() {
        assert_eq!(parse_value("1000000").unwrap(), "0xf4240");
        assert_eq!(parse_value("0").unwrap(), "0x0");
    }

    #[test]
    fn test_parse_value_ether() {
        assert_eq!(parse_value("1ether").unwrap(), "0xde0b6b3a7640000");
        assert_eq!(parse_value("1.ether").unwrap(), "0xde0b6b3a7640000");
        assert_eq!(parse_value("0.000000000000000001ether").unwrap(), "0x1");

        let result = parse_value("1.010101010101010101ether").unwrap();
        let expected = U256::from(1_010_101_010_101_010_101u128);
        assert_eq!(result, format!("{expected:#x}"));
    }

    #[test]
    fn test_parse_value_gwei() {
        assert_eq!(parse_value("1gwei").unwrap(), "0x3b9aca00");
        assert_eq!(parse_value("100gwei").unwrap(), "0x174876e800");
    }

    #[test]
    fn test_parse_value_wei() {
        assert_eq!(parse_value("1wei").unwrap(), "0x1");
        assert_eq!(parse_value("1000000000wei").unwrap(), "0x3b9aca00");
    }

    #[test]
    fn test_hex_uppercase_prefix() {
        assert_eq!(
            parse_value("0Xde0b6b3a7640000").unwrap(),
            "0xde0b6b3a7640000"
        );
    }

    #[test]
    fn test_parse_value_invalid() {
        assert!(parse_value("abc").is_err());
        assert!(parse_value("1.2.3ether").is_err());
    }
}
