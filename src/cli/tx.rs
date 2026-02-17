use super::trace::{self, TraceError, TraceOpts};
use clap::Parser;
use serde_json::json;

#[derive(Parser, Debug)]
pub struct TxArgs {
    /// Transaction hash to trace (0x-prefixed).
    pub tx_hash: String,

    #[command(flatten)]
    pub opts: TraceOpts,
}

pub fn run(args: TxArgs) -> Result<(), TraceError> {
    trace::validate_hex(&args.tx_hash, "tx_hash")?;
    if args.tx_hash.len() != 66 {
        return Err(TraceError::InvalidInput(format!(
            "tx_hash: expected 64 hex chars, got {}",
            args.tx_hash.len() - 2
        )));
    }

    let payload = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "debug_traceTransaction",
        "params": [
            args.tx_hash,
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
