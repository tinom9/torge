use crate::utils::disk_cache::{CacheError, DiskCache, ALL_CACHE_KINDS};
use clap::Parser;
use std::fs;
use std::path::Path;

#[derive(Parser, Debug)]
pub struct CleanArgs {
    /// Only remove unknown (unresolved) entries from the cache.
    #[arg(long)]
    pub only_unknown: bool,
}

pub fn run(args: &CleanArgs) -> Result<(), CacheError> {
    let mut found_any = false;

    for kind in ALL_CACHE_KINDS {
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
        match DiskCache::cache_path(ALL_CACHE_KINDS[0]).and_then(|p| p.parent().map(Path::to_owned))
        {
            Some(dir) => println!("no cache files found in {}", dir.display()),
            None => println!("no cache files found (could not determine cache directory)"),
        }
    }

    Ok(())
}
