mod cli;
mod utils;

use clap::Parser;

fn main() {
    let _ = dotenvy::dotenv();

    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = cli::Args::parse();

    match args.command {
        cli::Command::Tx(tx_args) => cli::tx::run(tx_args)?,
        cli::Command::Call(call_args) => cli::call::run(call_args)?,
        cli::Command::Clean(ref clean_args) => cli::clean::run(clean_args)?,
    }

    Ok(())
}
