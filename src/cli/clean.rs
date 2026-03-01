use crate::utils::disk_cache::{CacheError, DiskCache};
use clap::Parser;
use std::fs;
use thiserror::Error;

const CACHE_KINDS: &[&str] = &["selectors", "contracts"];

#[derive(Parser, Debug)]
pub struct CleanArgs {
    /// Only remove unknown (unresolved) entries from the cache.
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
    let mut found_any = false;

    for kind in CACHE_KINDS {
        let Some(path) = DiskCache::cache_path(kind) else {
            continue;
        };
        if !path.exists() {
            continue;
        }

        found_any = true;

        if args.only_unknown {
            let (kept, removed) = DiskCache::remove_unknown(kind)?;
            println!(
                "{kind}: removed {removed} unknown, kept {kept} — {}",
                path.display()
            );
        } else {
            fs::remove_file(&path)?;
            println!("{kind}: cache cleared — {}", path.display());
        }
    }

    if !found_any {
        match DiskCache::cache_path(CACHE_KINDS[0]).and_then(|p| p.parent().map(|d| d.to_owned())) {
            Some(dir) => println!("no cache files found in {}", dir.display()),
            None => println!("no cache files found (could not determine cache directory)"),
        }
    }

    Ok(())
}
