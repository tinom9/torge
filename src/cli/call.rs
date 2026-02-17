use super::trace::{self, TraceError, TraceOpts};
use crate::utils::{selector_resolver::SelectorResolver, value_parser};
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

    /// Gas limit for the simulated call (hex, e.g. 0x5f5e100).
    #[arg(long)]
    pub gas_limit: Option<String>,

    /// ETH value to send with the call.
    ///
    /// Supports hex (0x1234), decimal (1000000), and units
    /// (1gwei, 100gwei, 1ether, 4.5ether, 0.01ether).
    #[arg(long)]
    pub value: Option<String>,

    #[command(flatten)]
    pub opts: TraceOpts,
}

pub fn run(args: CallArgs) -> Result<(), TraceError> {
    if args.opts.include_args && !args.opts.resolve_selectors {
        return Err(TraceError::IncludeArgsRequiresResolveSelectors);
    }

    let rpc_url = trace::resolve_rpc_url(args.opts.rpc_url)?;
    let client = trace::create_client()?;

    let mut tx_object = json!({
        "to": args.to,
        "data": args.data,
    });

    if let Some(from) = &args.from {
        tx_object["from"] = json!(from);
    }
    if let Some(gas) = &args.gas_limit {
        tx_object["gas"] = json!(gas);
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
            "latest",
            {
                "tracer": "callTracer",
                "tracerConfig": {
                    "onlyTopCall": false,
                    "withLog": args.opts.include_logs,
                }
            }
        ]
    });

    let resp = client
        .post(&rpc_url)
        .json(&payload)
        .send()?
        .error_for_status()?;

    let call_trace = trace::parse_rpc_response(resp)?;

    let mut resolver = SelectorResolver::new(&client, args.opts.resolve_selectors);
    trace::print_trace(
        &call_trace,
        &mut resolver,
        args.opts.include_args,
        args.opts.include_calldata,
        args.opts.include_logs,
    );

    Ok(())
}
