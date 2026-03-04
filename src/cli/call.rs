use super::trace::{self, TraceError, TraceOpts};
use crate::utils::{hex_utils, value_parser};
use clap::Parser;
use serde_json::json;

/// Positional arguments: `[TO] <DATA>`.
///
/// - With `--create`: expects 1 arg (DATA only).
/// - Without `--create`: expects 2 args (TO and DATA).
#[derive(Parser, Debug)]
#[command(
    override_usage = "torge call [OPTIONS] [TO] <DATA>\n       torge call [OPTIONS] --create <DATA>"
)]
pub struct CallArgs {
    /// Positional arguments: `[TO] <DATA>` or `<DATA>` with `--create`.
    #[arg(num_args = 1..=2, value_name = "[TO] <DATA>")]
    pub args: Vec<String>,

    /// Simulate a contract creation transaction (no `to` address).
    #[arg(long)]
    pub create: bool,

    /// Sender address for the simulated call.
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

/// Parsed positional arguments for the call command.
struct ParsedArgs {
    to: Option<String>,
    data: String,
}

/// Parse positional arguments based on count and `--create` flag.
///
/// - With `--create`: expects 1 arg (DATA).
/// - Without `--create`: expects 2 args (TO, DATA).
fn parse_positional_args(args: &CallArgs) -> Result<ParsedArgs, TraceError> {
    match (args.create, args.args.len()) {
        (true, 1) => Ok(ParsedArgs {
            to: None,
            data: args.args[0].clone(),
        }),
        (true, 2) => Err(TraceError::InvalidInput(
            "--create cannot be used with a TO address".to_string(),
        )),
        (true, _) => Err(TraceError::InvalidInput(
            "expected exactly 1 argument (DATA) when using --create".to_string(),
        )),
        (false, 2) => Ok(ParsedArgs {
            to: Some(args.args[0].clone()),
            data: args.args[1].clone(),
        }),
        (false, 1) => Err(TraceError::InvalidInput(
            "missing DATA argument; use --create for contract creation or provide both TO and DATA"
                .to_string(),
        )),
        (false, _) => Err(TraceError::InvalidInput(
            "expected 2 arguments: TO and DATA".to_string(),
        )),
    }
}

pub fn run(args: CallArgs) -> Result<(), TraceError> {
    let parsed = parse_positional_args(&args)?;

    if let Some(to) = &parsed.to {
        trace::validate_address(to, "TO")?;
    }
    trace::validate_hex(&parsed.data, "DATA")?;
    if let Some(from) = &args.from {
        trace::validate_address(from, "--from")?;
    }
    let block_id = parse_block_id(&args.block)?;

    let mut tx_object = json!({
        "data": parsed.data,
    });

    if let Some(to) = &parsed.to {
        tx_object["to"] = json!(to);
    }
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

    let payload = trace::rpc_payload(
        1,
        "debug_traceCall",
        json!([
            &tx_object,
            &block_id,
            trace::call_tracer_config(args.opts.include_logs)
        ]),
    );

    let prestate_payload = args.opts.include_storage.then(|| {
        trace::rpc_payload(
            2,
            "debug_traceCall",
            json!([&tx_object, &block_id, trace::prestate_tracer_config()]),
        )
    });

    trace::execute_and_print(&payload, prestate_payload.as_ref(), args.opts)
}

/// Parse a block identifier from user input into a JSON-RPC block parameter.
///
/// Accepts named tags (`latest`, `earliest`, `pending`, `safe`, `finalized`),
/// hex block numbers (`0xBC614E`), or decimal block numbers (`12345678`).
fn parse_block_id(block: &str) -> Result<String, TraceError> {
    match block {
        "latest" | "earliest" | "pending" | "safe" | "finalized" => Ok(block.to_string()),
        s if hex_utils::require_0x(s).is_some() => {
            let hex = hex_utils::require_0x(s).unwrap();
            if hex.is_empty() || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(TraceError::InvalidInput(format!(
                    "--block: invalid hex block number '{s}'"
                )));
            }
            Ok(format!("0x{}", hex.to_ascii_lowercase()))
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

    fn make_call_args(args: Vec<&str>, create: bool) -> CallArgs {
        CallArgs {
            args: args.into_iter().map(String::from).collect(),
            create,
            from: None,
            gas_limit: None,
            value: None,
            block: "latest".to_string(),
            opts: TraceOpts {
                rpc_url: None,
                resolve_selectors: false,
                resolve_contracts: false,
                include_args: false,
                include_calldata: false,
                include_logs: false,
                include_storage: false,
                no_proxy: false,
                no_color: false,
            },
        }
    }

    #[test]
    fn test_parse_positional_args_create_one_arg() {
        let args = make_call_args(vec!["0xdead"], true);
        let parsed = parse_positional_args(&args).unwrap();
        assert!(parsed.to.is_none());
        assert_eq!(parsed.data, "0xdead");
    }

    #[test]
    fn test_parse_positional_args_create_two_args() {
        let args = make_call_args(vec!["0xaddr", "0xdata"], true);
        let Err(err) = parse_positional_args(&args) else {
            panic!("expected Err");
        };
        assert!(err.to_string().contains("--create cannot be used"));
    }

    #[test]
    fn test_parse_positional_args_create_no_args() {
        let args = make_call_args(vec![], true);
        let Err(err) = parse_positional_args(&args) else {
            panic!("expected Err");
        };
        assert!(err.to_string().contains("expected exactly 1 argument"));
    }

    #[test]
    fn test_parse_positional_args_create_three_args() {
        let args = make_call_args(vec!["a", "b", "c"], true);
        let Err(err) = parse_positional_args(&args) else {
            panic!("expected Err");
        };
        assert!(err.to_string().contains("expected exactly 1 argument"));
    }

    #[test]
    fn test_parse_positional_args_normal_two_args() {
        let args = make_call_args(vec!["0xaddr", "0xdata"], false);
        let parsed = parse_positional_args(&args).unwrap();
        assert_eq!(parsed.to, Some("0xaddr".to_string()));
        assert_eq!(parsed.data, "0xdata");
    }

    #[test]
    fn test_parse_positional_args_normal_one_arg() {
        let args = make_call_args(vec!["0xdata"], false);
        let Err(err) = parse_positional_args(&args) else {
            panic!("expected Err");
        };
        assert!(err.to_string().contains("missing DATA argument"));
    }

    #[test]
    fn test_parse_positional_args_normal_no_args() {
        let args = make_call_args(vec![], false);
        let Err(err) = parse_positional_args(&args) else {
            panic!("expected Err");
        };
        assert!(err.to_string().contains("expected 2 arguments"));
    }

    #[test]
    fn test_parse_positional_args_normal_three_args() {
        let args = make_call_args(vec!["a", "b", "c"], false);
        let Err(err) = parse_positional_args(&args) else {
            panic!("expected Err");
        };
        assert!(err.to_string().contains("expected 2 arguments"));
    }

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
        assert_eq!(parse_block_id("0xBC614E").unwrap(), "0xbc614e");
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
