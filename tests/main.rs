use assert_cmd::prelude::*;
use mockito::Server;
use predicates::prelude::*;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

#[derive(Debug, Deserialize)]
struct TestCaseRaw {
    name: String,
    tx_hash: String,
    rpc_response: serde_json::Value,
    expected_trace: String,
    expected_trace_logs: Option<String>,
    expected_trace_full: Option<String>,
}

#[derive(Debug)]
struct TestCase {
    name: String,
    tx_hash: String,
    rpc_response: serde_json::Value,
    expected_trace: String,
    expected_trace_logs: Option<String>,
    expected_trace_full: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SelectorsFixture {
    selectors: HashMap<String, String>,
}

fn load_test_fixtures() -> Vec<TestCase> {
    let fixtures_path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/main.json");
    let traces_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/traces");

    let fixtures_content =
        std::fs::read_to_string(fixtures_path).expect("Failed to read fixtures/main.json");

    let raw_cases: Vec<TestCaseRaw> =
        serde_json::from_str(&fixtures_content).expect("Failed to parse fixtures/main.json");

    // Load trace files and convert to `TestCase`.
    raw_cases
        .into_iter()
        .map(|raw| {
            let expected_trace_path = PathBuf::from(traces_dir).join(&raw.expected_trace);
            let expected_trace = std::fs::read_to_string(&expected_trace_path)
                .unwrap_or_else(|_| panic!("Failed to read trace file: {:?}", expected_trace_path));

            let expected_trace_logs = raw.expected_trace_logs.map(|filename| {
                let logs_trace_path = PathBuf::from(traces_dir).join(&filename);
                std::fs::read_to_string(&logs_trace_path).unwrap_or_else(|_| {
                    panic!("Failed to read logs trace file: {:?}", logs_trace_path)
                })
            });

            let expected_trace_full = raw.expected_trace_full.map(|filename| {
                let full_trace_path = PathBuf::from(traces_dir).join(&filename);
                std::fs::read_to_string(&full_trace_path).unwrap_or_else(|_| {
                    panic!("Failed to read full trace file: {:?}", full_trace_path)
                })
            });

            TestCase {
                name: raw.name,
                tx_hash: raw.tx_hash,
                rpc_response: raw.rpc_response,
                expected_trace,
                expected_trace_logs,
                expected_trace_full,
            }
        })
        .collect()
}

fn load_selectors() -> HashMap<String, String> {
    let selectors_path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/selectors.json");
    let selectors_content =
        std::fs::read_to_string(selectors_path).expect("Failed to read fixtures/selectors.json");
    let fixture: SelectorsFixture =
        serde_json::from_str(&selectors_content).expect("Failed to parse fixtures/selectors.json");
    fixture.selectors
}

fn setup_sourcify_mock(
    server: &mut Server,
    selectors: &HashMap<String, String>,
) -> Vec<mockito::Mock> {
    // Create a mock for each selector and return them to keep them alive.
    selectors
        .iter()
        .flat_map(|(selector, signature)| {
            // If signature is <UNKNOWN>, return empty results (selector not found).
            let is_event = selector.len() == 66; // Event topic0 is 32 bytes = 66 chars with 0x prefix.

            let response = if signature == "<UNKNOWN>" {
                if is_event {
                    json!({
                        "ok": true,
                        "result": {
                            "event": {}
                        }
                    })
                } else {
                    json!({
                        "ok": true,
                        "result": {
                            "function": {}
                        }
                    })
                }
            } else if is_event {
                json!({
                    "ok": true,
                    "result": {
                        "event": {
                            selector: [{
                                "name": signature,
                                "filtered": false,
                                "hasVerifiedContract": true
                            }]
                        }
                    }
                })
            } else {
                json!({
                    "ok": true,
                    "result": {
                        "function": {
                            selector: [{
                                "name": signature,
                                "filtered": false,
                                "hasVerifiedContract": true
                            }]
                        }
                    }
                })
            };

            let endpoint = if is_event {
                format!(
                    "/signature-database/v1/lookup?event={}&filter=false",
                    selector
                )
            } else {
                format!(
                    "/signature-database/v1/lookup?function={}&filter=false",
                    selector
                )
            };

            vec![server
                .mock("GET", endpoint.as_str())
                .with_status(200)
                .with_header("content-type", "application/json")
                .with_body(response.to_string())
                .create()]
        })
        .collect()
}

/// Test mode configuration.
#[derive(Debug, Clone, Copy)]
enum TestMode {
    Basic,
    Logs,
    Full,
}

impl TestMode {
    fn name(&self) -> &'static str {
        match self {
            TestMode::Basic => "basic",
            TestMode::Logs => "logs",
            TestMode::Full => "full",
        }
    }
}

/// Helper function to run a trace test with optional selector resolution.
fn run_trace_test(test_case: &TestCase, expected_output: &str, mode: TestMode) {
    println!(
        "\n=== Running {} test case: {} ===",
        mode.name(),
        test_case.name
    );

    // Create mock RPC server.
    let mut rpc_server = Server::new();
    let _rpc_mock = rpc_server
        .mock("POST", "/")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(serde_json::to_string(&test_case.rpc_response).unwrap())
        .create();

    // Optionally create Sourcify mock server.
    let mut sourcify_server = Server::new();
    let _sourcify_mocks = if matches!(mode, TestMode::Full) {
        let selectors = load_selectors();
        Some(setup_sourcify_mock(&mut sourcify_server, &selectors))
    } else {
        None
    };

    // Build and run the binary.
    let binary_path = assert_cmd::cargo::cargo_bin!("torge");
    let mut cmd = Command::new(binary_path);

    cmd.arg("tx")
        .arg(&test_case.tx_hash)
        .arg("-r")
        .arg(rpc_server.url())
        .env("TORGE_DISABLE_CACHE", "1");

    // Configure based on test mode.
    match mode {
        TestMode::Basic => {
            // No extra flags.
        }
        TestMode::Logs => {
            cmd.arg("--include-logs");
        }
        TestMode::Full => {
            cmd.arg("--include-logs")
                .arg("--resolve-selectors")
                .arg("--include-args")
                .arg("--include-calldata")
                .env("SOURCIFY_URL", format!("{}/", sourcify_server.url()));
        }
    }

    // Assert the output contains the expected trace.
    let result = cmd.assert().success();

    // When debugging, uncomment:
    // let stdout = String::from_utf8_lossy(&result.get_output().stdout);
    // let filename = format!("{}-{}.test.log", test_case.name.replace("::", "-"), mode.name());
    // std::fs::write(&filename, stdout.as_ref()).expect("Failed to write test output");
    // println!("Wrote output to {}", filename);

    result.stdout(predicate::str::contains(expected_output));

    println!("✓ {} test case '{}' passed", mode.name(), test_case.name);
}

#[test]
fn test_trace_outputs() {
    let test_cases = load_test_fixtures();

    for test_case in test_cases {
        run_trace_test(&test_case, &test_case.expected_trace, TestMode::Basic);
    }
}

#[test]
fn test_logs_trace_outputs() {
    let test_cases = load_test_fixtures();

    for test_case in test_cases {
        // Skip test cases that don't have expected_trace_logs.
        if let Some(expected_logs) = &test_case.expected_trace_logs {
            run_trace_test(&test_case, expected_logs, TestMode::Logs);
        } else {
            println!(
                "⊘ Skipping test case '{}' (no expected_trace_logs)",
                test_case.name
            );
        }
    }
}

#[test]
fn test_full_trace_outputs() {
    let test_cases = load_test_fixtures();

    for test_case in test_cases {
        // Skip test cases that don't have expected_trace_full.
        if let Some(expected_full) = &test_case.expected_trace_full {
            run_trace_test(&test_case, expected_full, TestMode::Full);
        } else {
            println!(
                "⊘ Skipping test case '{}' (no expected_trace_full)",
                test_case.name
            );
        }
    }
}

#[test]
fn test_clean_only_unknown() {
    let tmp = TempDir::new().unwrap();
    let cache_path = tmp.path().join("torge").join("selectors.json");
    std::fs::create_dir_all(cache_path.parent().unwrap()).unwrap();

    // Write a cache with mixed known and unknown entries.
    let cache = json!({
        "selectors": {
            "0xa9059cbb": "transfer(address,uint256)",
            "0x70a08231": "balanceOf(address)",
            "0xdeadbeef": "<UNKNOWN>",
            "0xcafebabe": "<UNKNOWN>"
        }
    });
    std::fs::write(&cache_path, serde_json::to_string_pretty(&cache).unwrap()).unwrap();

    let binary_path = assert_cmd::cargo::cargo_bin!("torge");
    let mut cmd = Command::new(binary_path);
    cmd.arg("clean")
        .arg("--only-unknown")
        .env("XDG_CACHE_HOME", tmp.path());

    cmd.assert().success().stdout(predicate::str::contains(
        "removed 2 unknown selector(s), kept 2",
    ));

    // Verify the cache file still exists with only known entries.
    let remaining: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&cache_path).unwrap()).unwrap();
    let selectors = remaining["selectors"].as_object().unwrap();
    assert_eq!(selectors.len(), 2);
    assert!(selectors.contains_key("0xa9059cbb"));
    assert!(selectors.contains_key("0x70a08231"));
    assert!(!selectors.contains_key("0xdeadbeef"));
    assert!(!selectors.contains_key("0xcafebabe"));
}

#[test]
fn test_clean_removes_entire_cache() {
    let tmp = TempDir::new().unwrap();
    let cache_path = tmp.path().join("torge").join("selectors.json");
    std::fs::create_dir_all(cache_path.parent().unwrap()).unwrap();

    let cache = json!({
        "selectors": {
            "0xa9059cbb": "transfer(address,uint256)",
            "0xdeadbeef": "<UNKNOWN>"
        }
    });
    std::fs::write(&cache_path, serde_json::to_string_pretty(&cache).unwrap()).unwrap();

    let binary_path = assert_cmd::cargo::cargo_bin!("torge");
    let mut cmd = Command::new(binary_path);
    cmd.arg("clean").env("XDG_CACHE_HOME", tmp.path());

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("cache cleared"));

    assert!(!cache_path.exists());
}
