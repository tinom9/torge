use crate::utils::disk_cache;
use std::fs;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CleanError {
    #[error("io error: {0}")]
    Io(String),
}

pub fn run() -> Result<(), CleanError> {
    match disk_cache::DiskCache::cache_path() {
        Some(path) => {
            if path.exists() {
                fs::remove_file(&path)
                    .map_err(|e| CleanError::Io(format!("failed to remove cache file: {e}")))?;
                println!("cache cleared: {}", path.display());
            } else {
                println!("no cache file found at: {}", path.display());
            }
        }
        None => {
            println!("could not determine cache directory");
        }
    }
    Ok(())
}
