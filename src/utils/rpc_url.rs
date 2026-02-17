use std::{env, fs};
use thiserror::Error;

const FOUNDRY_CONFIG: &str = "foundry.toml";
const VAR_START: &str = "${";
const VAR_END: char = '}';

#[derive(Debug, Error)]
pub enum RpcUrlError {
    #[error("rpc alias '{0}' not found in foundry.toml")]
    AliasNotFound(String),

    #[error("invalid RPC URL '{url}' resolved from alias '{alias}'\n  resolved to: {resolved}\n  hint: {hint}")]
    InvalidResolvedUrl {
        alias: String,
        url: String,
        resolved: String,
        hint: String,
    },

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("toml parse error: {0}")]
    TomlParse(#[from] toml::de::Error),
}

/// Resolve an RPC URL from a string that may be a direct URL or a Foundry alias.
///
/// Returns the resolved URL. If the input starts with `http://` or `https://`,
/// it's returned as-is. Otherwise, it's treated as an alias and resolved from `foundry.toml`.
pub fn resolve(url_or_alias: &str) -> Result<String, RpcUrlError> {
    if is_url(url_or_alias) {
        return Ok(url_or_alias.to_string());
    }

    let resolved = resolve_alias(url_or_alias)?;
    validate_resolved_url(&resolved, url_or_alias)?;
    Ok(resolved)
}

/// Check if a string is a URL (starts with `http://` or `https://`).
fn is_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

/// Resolve an RPC alias from `foundry.toml` in the current directory.
fn resolve_alias(alias: &str) -> Result<String, RpcUrlError> {
    let contents = fs::read_to_string(FOUNDRY_CONFIG)
        .map_err(|_| RpcUrlError::AliasNotFound(alias.to_string()))?;

    let config: toml::Value = toml::from_str(&contents)?;

    // Support both string and table formats:
    //   alias = "https://..."           (string)
    //   alias = { url = "https://..." } (table, as used by Foundry)
    let url = config
        .get("rpc_endpoints")
        .and_then(|endpoints| endpoints.get(alias))
        .and_then(|v| v.as_str().or_else(|| v.get("url").and_then(|u| u.as_str())))
        .ok_or_else(|| RpcUrlError::AliasNotFound(alias.to_string()))?;

    Ok(substitute_env_vars(url))
}

/// Substitute environment variables in a string using the format `${VAR_NAME}`.
fn substitute_env_vars(s: &str) -> String {
    substitute_vars(s, |name| env::var(name).ok())
}

/// Core variable substitution logic, parameterized by a resolver function.
///
/// Advances the scan position past each replacement to avoid re-processing
/// substituted text (prevents infinite loops if a value contains `${`).
fn substitute_vars(s: &str, resolve_var: impl Fn(&str) -> Option<String>) -> String {
    let mut result = s.to_string();
    let var_start_len = VAR_START.len();
    let mut pos = 0;

    while pos < result.len() {
        let Some(rel) = result[pos..].find(VAR_START) else {
            break;
        };
        let start = pos + rel;

        let Some(end_offset) = result[start..].find(VAR_END) else {
            break;
        };

        let var_name = &result[start + var_start_len..start + end_offset];
        let replacement = resolve_var(var_name).unwrap_or_default();
        let replacement_len = replacement.len();
        result.replace_range(start..=start + end_offset, &replacement);
        pos = start + replacement_len;
    }
    result
}

/// Find unresolved environment variables in a string.
fn find_unresolved_vars(s: &str) -> Vec<String> {
    find_vars(s, |name| env::var(name).is_err())
}

/// Core unresolved-variable detection, parameterized by a predicate.
fn find_vars(s: &str, is_unresolved: impl Fn(&str) -> bool) -> Vec<String> {
    let var_start_len = VAR_START.len();
    let mut vars = Vec::new();
    let mut pos = 0;

    while let Some(start_offset) = s[pos..].find(VAR_START) {
        let start = pos + start_offset;
        let Some(end_offset) = s[start..].find(VAR_END) else {
            break;
        };

        let var_name = &s[start + var_start_len..start + end_offset];
        if is_unresolved(var_name) {
            vars.push(var_name.to_string());
        }
        pos = start + end_offset + 1;
    }
    vars
}

/// Validate that an RPC URL is well-formed and provide helpful error messages.
fn validate_resolved_url(url: &str, alias: &str) -> Result<(), RpcUrlError> {
    if url.trim().is_empty() {
        let hint = get_raw_url_from_foundry(alias)
            .and_then(|raw| {
                let missing = find_unresolved_vars(&raw);
                (!missing.is_empty()).then(|| {
                    format!(
                        "environment variable(s) not set: {}. check your .env file or export them",
                        missing.join(", ")
                    )
                })
            })
            .unwrap_or_else(|| {
                format!(
                    "the alias resolved to an empty URL. check your {FOUNDRY_CONFIG} configuration"
                )
            });

        return Err(RpcUrlError::InvalidResolvedUrl {
            alias: alias.to_string(),
            url: url.to_string(),
            resolved: url.to_string(),
            hint,
        });
    }

    if !is_url(url) {
        return Err(RpcUrlError::InvalidResolvedUrl {
            alias: alias.to_string(),
            url: url.to_string(),
            resolved: url.to_string(),
            hint: format!("URL must start with http:// or https://. got: {url}"),
        });
    }

    Ok(())
}

/// Helper to get the raw (unsubstituted) URL from `foundry.toml` for error messages.
fn get_raw_url_from_foundry(alias: &str) -> Option<String> {
    let contents = fs::read_to_string(FOUNDRY_CONFIG).ok()?;
    let config: toml::Value = toml::from_str(&contents).ok()?;
    let entry = config.get("rpc_endpoints")?.get(alias)?;
    entry
        .as_str()
        .or_else(|| entry.get("url").and_then(|u| u.as_str()))
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_is_url() {
        assert!(is_url("http://localhost:8545"));
        assert!(is_url("https://mainnet.infura.io"));
        assert!(!is_url("ethereum"));
        assert!(!is_url("mainnet"));
        assert!(!is_url("localhost:8545"));
    }

    #[test]
    fn test_substitute_vars() {
        let vars: HashMap<&str, &str> =
            [("TEST_VAR", "test_value"), ("ANOTHER_VAR", "another_value")].into();
        let resolve = |name: &str| vars.get(name).map(|v| v.to_string());

        assert_eq!(
            substitute_vars("http://${TEST_VAR}/api", &resolve),
            "http://test_value/api"
        );
        assert_eq!(
            substitute_vars("${TEST_VAR}:${ANOTHER_VAR}", &resolve),
            "test_value:another_value"
        );
        assert_eq!(substitute_vars("no_vars_here", &resolve), "no_vars_here");
        assert_eq!(substitute_vars("${NONEXISTENT_VAR}", &resolve), "");

        // Replacement containing ${...} must not be re-processed (no infinite loop).
        let tricky: HashMap<&str, &str> = [("VAR", "has${VAR}inside")].into();
        let tricky_resolve = |name: &str| tricky.get(name).map(|v| v.to_string());
        assert_eq!(
            substitute_vars("pre${VAR}post", &tricky_resolve),
            "prehas${VAR}insidepost"
        );
    }

    #[test]
    fn test_find_vars() {
        let resolved: &[&str] = &["TEST_RESOLVED_VAR"];
        let is_unresolved = |name: &str| !resolved.contains(&name);

        let vars = find_vars("http://${TEST_UNRESOLVED_VAR}/api", &is_unresolved);
        assert_eq!(vars, vec!["TEST_UNRESOLVED_VAR"]);

        let vars = find_vars("http://${TEST_RESOLVED_VAR}/api", &is_unresolved);
        assert!(vars.is_empty());

        let vars = find_vars("${VAR1}:${VAR2}", &is_unresolved);
        assert_eq!(vars, vec!["VAR1", "VAR2"]);
    }

    #[test]
    fn test_resolve_http() {
        assert_eq!(
            resolve("http://localhost:8545").unwrap(),
            "http://localhost:8545"
        );
    }

    #[test]
    fn test_resolve_https() {
        assert_eq!(
            resolve("https://mainnet.infura.io").unwrap(),
            "https://mainnet.infura.io"
        );
    }
}
