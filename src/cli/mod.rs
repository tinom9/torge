pub mod clean;
pub mod tx;

use clap::Parser;

/// Simple CLI to run `debug_traceTransaction` and print a gas trace.
#[derive(Parser, Debug)]
#[command(author, version, about)]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Parser, Debug)]
pub enum Command {
    /// Trace an Ethereum transaction.
    Tx(tx::TxArgs),

    /// Clean the selector cache.
    Clean(clean::CleanArgs),
}
