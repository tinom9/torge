use super::trace::{self, TraceError, TraceOpts};
use crate::utils::selector_resolver::SelectorResolver;
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
    if args.opts.include_args && !args.opts.resolve_selectors {
        return Err(TraceError::IncludeArgsRequiresResolveSelectors);
    }

    let rpc_url = trace::resolve_rpc_url(args.opts.rpc_url)?;
    let client = trace::create_client()?;

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
