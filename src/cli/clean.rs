use crate::utils::disk_cache::{CacheError, DiskCache};
use clap::Parser;
use std::fs;
use thiserror::Error;

#[derive(Parser, Debug)]
pub struct CleanArgs {
    /// Only remove unknown (unresolved) selectors from the cache.
    #[arg(long)]
    pub only_unknown: bool,
}

#[derive(Debug, Error)]
pub enum CleanError {
    #[error("{0}")]
    Cache(#[from] CacheError),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

#[allow(clippy::needless_pass_by_value)] // clap produces owned values
pub fn run(args: CleanArgs) -> Result<(), CleanError> {
    let Some(path) = DiskCache::cache_path() else {
        println!("could not determine cache directory");
        return Ok(());
    };

    if !path.exists() {
        println!("no cache file found at: {}", path.display());
        return Ok(());
    }

    if args.only_unknown {
        let (kept, removed) = DiskCache::remove_unknown()?;
        println!(
            "removed {removed} unknown selector(s), kept {kept} — {}",
            path.display()
        );
    } else {
        fs::remove_file(&path)?;
        println!("cache cleared: {}", path.display());
    }

    Ok(())
}
