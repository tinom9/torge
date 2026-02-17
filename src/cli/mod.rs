pub mod call;
pub mod clean;
pub mod trace;
pub mod tx;

use clap::Parser;

/// Simple CLI to run `debug_traceTransaction` and `debug_traceCall` and print a gas trace.
#[derive(Parser, Debug)]
#[command(author, version, about)]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Parser, Debug)]
pub enum Command {
    /// Trace an Ethereum transaction (`debug_traceTransaction`).
    Tx(tx::TxArgs),

    /// Trace an Ethereum call (`debug_traceCall`).
    Call(call::CallArgs),

    /// Clean the selector cache.
    Clean(clean::CleanArgs),
}
