/// Check if an address is a precompiled contract and return its name and signature.
///
/// Precompiled contracts are at addresses 0x01-0x0a (and potentially higher in newer forks).
/// Returns `Some((name, signature))` if the address is a known precompile, `None` otherwise.
pub fn get_precompile_info(address: &str) -> Option<(&'static str, &'static str)> {
    let addr_lower = address.to_lowercase();
    let addr = addr_lower.strip_prefix("0x").unwrap_or(&addr_lower);

    // Handle both full addresses (40 chars) and short forms.
    let normalized = if addr.len() == 40 {
        &addr[24..] // Take last 16 chars (8 bytes) for comparison.
    } else {
        addr
    };

    match normalized {
        "0000000000000001" | "01" | "1" => {
            // ecrecover takes 128 bytes: hash(32) + v(32) + r(32) + s(32).
            Some(("ecrecover", "ecrecover(bytes32,uint8,uint256,uint256)"))
        }
        "0000000000000002" | "02" | "2" => Some(("sha256", "sha256(bytes)")),
        "0000000000000003" | "03" | "3" => Some(("ripemd160", "ripemd160(bytes)")),
        "0000000000000004" | "04" | "4" => Some(("identity", "identity(bytes)")),
        "0000000000000005" | "05" | "5" => {
            Some(("modexp", "modexp(uint256,uint256,uint256,bytes)"))
        }
        "0000000000000006" | "06" | "6" => Some(("ecadd", "ecadd(bytes)")),
        "0000000000000007" | "07" | "7" => Some(("ecmul", "ecmul(bytes)")),
        "0000000000000008" | "08" | "8" => Some(("ecpairing", "ecpairing(bytes)")),
        "0000000000000009" | "09" | "9" => Some(("blake2f", "blake2f(bytes)")),
        "000000000000000a" | "0a" | "a" => {
            Some(("pointevaluation", "pointevaluation(bytes)"))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ecrecover_full_address() {
        let result = get_precompile_info("0x0000000000000000000000000000000000000001");
        assert!(result.is_some());
        let (name, sig) = result.unwrap();
        assert_eq!(name, "ecrecover");
        assert_eq!(sig, "ecrecover(bytes32,uint8,uint256,uint256)");
    }

    #[test]
    fn test_ecrecover_short_form() {
        let result = get_precompile_info("0x01");
        assert!(result.is_some());
        let (name, _) = result.unwrap();
        assert_eq!(name, "ecrecover");
    }

    #[test]
    fn test_ecrecover_no_prefix() {
        let result = get_precompile_info("1");
        assert!(result.is_some());
        let (name, _) = result.unwrap();
        assert_eq!(name, "ecrecover");
    }

    #[test]
    fn test_sha256() {
        let result = get_precompile_info("0x0000000000000000000000000000000000000002");
        assert!(result.is_some());
        let (name, sig) = result.unwrap();
        assert_eq!(name, "sha256");
        assert_eq!(sig, "sha256(bytes)");
    }

    #[test]
    fn test_blake2f() {
        let result = get_precompile_info("0x0000000000000000000000000000000000000009");
        assert!(result.is_some());
        let (name, sig) = result.unwrap();
        assert_eq!(name, "blake2f");
        assert_eq!(sig, "blake2f(bytes)");
    }

    #[test]
    fn test_pointevaluation() {
        let result = get_precompile_info("0x000000000000000000000000000000000000000a");
        assert!(result.is_some());
        let (name, sig) = result.unwrap();
        assert_eq!(name, "pointevaluation");
        assert_eq!(sig, "pointevaluation(bytes)");
    }

    #[test]
    fn test_not_a_precompile() {
        let result = get_precompile_info("0x0000000000000000000000000000000000000000");
        assert!(result.is_none());

        let result = get_precompile_info("0x1234567890123456789012345678901234567890");
        assert!(result.is_none());
    }

    #[test]
    fn test_case_insensitive() {
        let result1 = get_precompile_info("0x000000000000000000000000000000000000000a");
        let result2 = get_precompile_info("0x000000000000000000000000000000000000000A");
        assert_eq!(result1, result2);
    }
}
