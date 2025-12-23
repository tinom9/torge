use std::{env, fs};
use thiserror::Error;

const FOUNDRY_CONFIG: &str = "foundry.toml";
const VAR_START: &str = "${";
const VAR_END: char = '}';

#[derive(Debug, Error)]
pub enum RpcUrlError {
    #[error("rpc alias '{0}' not found in {}", FOUNDRY_CONFIG)]
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

    let url = config
        .get("rpc_endpoints")
        .and_then(|endpoints| endpoints.get(alias))
        .and_then(|v| v.as_str())
        .ok_or_else(|| RpcUrlError::AliasNotFound(alias.to_string()))?;

    Ok(substitute_env_vars(url))
}

/// Substitute environment variables in a string using the format `${VAR_NAME}`.
fn substitute_env_vars(s: &str) -> String {
    let mut result = s.to_string();
    let var_start_len = VAR_START.len();

    while let Some(start) = result.find(VAR_START) {
        let Some(end_offset) = result[start..].find(VAR_END) else {
            break;
        };

        let var_name = &result[start + var_start_len..start + end_offset];
        let replacement = env::var(var_name).unwrap_or_default();
        result.replace_range(start..start + end_offset + 1, &replacement);
    }
    result
}

/// Find unresolved environment variables in a string.
fn find_unresolved_vars(s: &str) -> Vec<String> {
    let var_start_len = VAR_START.len();
    let mut vars = Vec::new();
    let mut pos = 0;

    while let Some(start_offset) = s[pos..].find(VAR_START) {
        let start = pos + start_offset;
        let Some(end_offset) = s[start..].find(VAR_END) else {
            break;
        };

        let var_name = &s[start + var_start_len..start + end_offset];
        if env::var(var_name).is_err() {
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
    config
        .get("rpc_endpoints")?
        .get(alias)?
        .as_str()
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    fn test_is_url() {
        assert!(is_url("http://localhost:8545"));
        assert!(is_url("https://mainnet.infura.io"));
        assert!(!is_url("ethereum"));
        assert!(!is_url("mainnet"));
        assert!(!is_url("localhost:8545"));
    }

    #[test]
    #[serial]
    fn test_substitute_env_vars() {
        env::set_var("TEST_VAR", "test_value");
        env::set_var("ANOTHER_VAR", "another_value");

        assert_eq!(
            substitute_env_vars("http://${TEST_VAR}/api"),
            "http://test_value/api"
        );
        assert_eq!(
            substitute_env_vars("${TEST_VAR}:${ANOTHER_VAR}"),
            "test_value:another_value"
        );
        assert_eq!(substitute_env_vars("no_vars_here"), "no_vars_here");
        assert_eq!(substitute_env_vars("${NONEXISTENT_VAR}"), "");

        env::remove_var("TEST_VAR");
        env::remove_var("ANOTHER_VAR");
    }

    #[test]
    #[serial]
    fn test_find_unresolved_vars() {
        env::remove_var("TEST_UNRESOLVED_VAR");
        env::set_var("TEST_RESOLVED_VAR", "resolved");

        let vars = find_unresolved_vars("http://${TEST_UNRESOLVED_VAR}/api");
        assert_eq!(vars, vec!["TEST_UNRESOLVED_VAR"]);

        let vars = find_unresolved_vars("http://${TEST_RESOLVED_VAR}/api");
        assert!(vars.is_empty());

        let vars = find_unresolved_vars("${VAR1}:${VAR2}");
        assert_eq!(vars, vec!["VAR1", "VAR2"]);

        env::remove_var("TEST_RESOLVED_VAR");
    }

    #[test]
    fn test_resolve_http() {
        let result = resolve("http://localhost:8545");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "http://localhost:8545");
    }

    #[test]
    fn test_resolve_https() {
        let result = resolve("https://mainnet.infura.io");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "https://mainnet.infura.io");
    }
}
