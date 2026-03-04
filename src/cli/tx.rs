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

    let TxArgs { tx_hash, opts } = args;

    let payload = trace::rpc_payload(
        1,
        "debug_traceTransaction",
        json!([&tx_hash, trace::call_tracer_config(opts.include_logs)]),
    );

    let prestate_payload = opts.include_storage.then(|| {
        trace::rpc_payload(
            2,
            "debug_traceTransaction",
            json!([&tx_hash, trace::prestate_tracer_config()]),
        )
    });

    trace::execute_and_print(&payload, prestate_payload.as_ref(), opts)
}
