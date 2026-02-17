use super::trace::{self, TraceError, TraceOpts};
use crate::utils::value_parser;
use clap::Parser;
use serde_json::json;

#[derive(Parser, Debug)]
pub struct CallArgs {
    /// Target contract address (0x-prefixed).
    pub to: String,

    /// Calldata hex (0x-prefixed).
    pub data: String,

    /// Sender address for the simulated call (defaults to zero address).
    #[arg(long)]
    pub from: Option<String>,

    /// Gas limit for the simulated call.
    ///
    /// Supports hex (`0x5f5e100`), decimal (`100000000`), and units
    /// (`1gwei`, `100gwei`).
    #[arg(long)]
    pub gas_limit: Option<String>,

    /// ETH value to send with the call.
    ///
    /// Supports hex (0x1234), decimal (1000000), and units
    /// (1gwei, 100gwei, 1ether, 4.5ether, 0.01ether).
    #[arg(long)]
    pub value: Option<String>,

    /// Block number or tag to simulate against (default: `latest`).
    ///
    /// Accepts tags (`latest`, `earliest`, `pending`, `safe`, `finalized`)
    /// or a block number (decimal `12345678` or hex `0xBC614E`).
    #[arg(long, default_value = "latest")]
    pub block: String,

    #[command(flatten)]
    pub opts: TraceOpts,
}

pub fn run(args: CallArgs) -> Result<(), TraceError> {
    trace::validate_address(&args.to, "--to")?;
    trace::validate_hex(&args.data, "--data")?;
    if let Some(from) = &args.from {
        trace::validate_address(from, "--from")?;
    }
    let block_id = parse_block_id(&args.block)?;

    let mut tx_object = json!({
        "to": args.to,
        "data": args.data,
    });

    if let Some(from) = &args.from {
        tx_object["from"] = json!(from);
    }
    if let Some(raw_gas) = &args.gas_limit {
        let hex_gas = value_parser::parse_value(raw_gas).map_err(TraceError::InvalidValue)?;
        tx_object["gas"] = json!(hex_gas);
    }
    if let Some(raw_value) = &args.value {
        let hex_value = value_parser::parse_value(raw_value).map_err(TraceError::InvalidValue)?;
        tx_object["value"] = json!(hex_value);
    }

    let payload = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "debug_traceCall",
        "params": [
            tx_object,
            block_id,
            {
                "tracer": "callTracer",
                "tracerConfig": {
                    "onlyTopCall": false,
                    "withLog": args.opts.include_logs,
                }
            }
        ]
    });

    trace::execute_and_print(&payload, args.opts)
}

/// Parse a block identifier from user input into a JSON-RPC block parameter.
///
/// Accepts named tags (`latest`, `earliest`, `pending`, `safe`, `finalized`),
/// hex block numbers (`0xBC614E`), or decimal block numbers (`12345678`).
fn parse_block_id(block: &str) -> Result<String, TraceError> {
    match block {
        "latest" | "earliest" | "pending" | "safe" | "finalized" => Ok(block.to_string()),
        s if s.starts_with("0x") || s.starts_with("0X") => {
            let hex = &s[2..];
            if hex.is_empty() || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(TraceError::InvalidInput(format!(
                    "--block: invalid hex block number '{s}'"
                )));
            }
            Ok(format!("0x{hex}"))
        }
        s => {
            let num: u64 = s.parse().map_err(|_| {
                TraceError::InvalidInput(format!("--block: invalid block identifier '{s}'"))
            })?;
            Ok(format!("0x{num:x}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_block_id_tags() {
        assert_eq!(parse_block_id("latest").unwrap(), "latest");
        assert_eq!(parse_block_id("earliest").unwrap(), "earliest");
        assert_eq!(parse_block_id("pending").unwrap(), "pending");
        assert_eq!(parse_block_id("safe").unwrap(), "safe");
        assert_eq!(parse_block_id("finalized").unwrap(), "finalized");
    }

    #[test]
    fn test_parse_block_id_decimal() {
        assert_eq!(parse_block_id("12345678").unwrap(), "0xbc614e");
        assert_eq!(parse_block_id("0").unwrap(), "0x0");
    }

    #[test]
    fn test_parse_block_id_hex() {
        assert_eq!(parse_block_id("0xBC614E").unwrap(), "0xBC614E");
        assert_eq!(parse_block_id("0x0").unwrap(), "0x0");
    }

    #[test]
    fn test_parse_block_id_invalid() {
        assert!(parse_block_id("abc").is_err());
        assert!(parse_block_id("-1").is_err());
        assert!(parse_block_id("0x").is_err());
        assert!(parse_block_id("0xGG").is_err());
    }
}
