use crate::utils::disk_cache::DiskCache;
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
    #[error("io error: {0}")]
    Io(String),
}

pub fn run(args: CleanArgs) -> Result<(), CleanError> {
    let path = match DiskCache::cache_path() {
        Some(p) => p,
        None => {
            println!("could not determine cache directory");
            return Ok(());
        }
    };

    if !path.exists() {
        println!("no cache file found at: {}", path.display());
        return Ok(());
    }

    if args.only_unknown {
        let (kept, removed) = DiskCache::remove_unknown()
            .map_err(|e| CleanError::Io(format!("failed to clean unknown entries: {e}")))?;
        println!(
            "removed {removed} unknown selector(s), kept {kept} — {}",
            path.display()
        );
    } else {
        fs::remove_file(&path)
            .map_err(|e| CleanError::Io(format!("failed to remove cache file: {e}")))?;
        println!("cache cleared: {}", path.display());
    }

    Ok(())
}
