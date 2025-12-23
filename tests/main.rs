use assert_cmd::prelude::*;
use mockito::Server;
use predicates::prelude::*;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Deserialize)]
struct TestCaseRaw {
    name: String,
    tx_hash: String,
    rpc_response: serde_json::Value,
    expected_trace: String,
    expected_trace_full: Option<String>,
}

#[derive(Debug)]
struct TestCase {
    name: String,
    tx_hash: String,
    rpc_response: serde_json::Value,
    expected_trace: String,
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
        .map(|(selector, signature)| {
            // If signature is <UNKNOWN>, return empty results (selector not found).
            let response = if signature == "<UNKNOWN>" {
                json!({
                    "ok": true,
                    "result": {
                        "function": {}
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

            server
                .mock(
                    "GET",
                    format!(
                        "/signature-database/v1/lookup?function={}&filter=false",
                        selector
                    )
                    .as_str(),
                )
                .with_status(200)
                .with_header("content-type", "application/json")
                .with_body(response.to_string())
                .create()
        })
        .collect()
}

/// Helper function to run a trace test with optional selector resolution.
fn run_trace_test(
    test_case: &TestCase,
    expected_output: &str,
    test_mode: &str,
    with_selectors: bool,
) {
    println!(
        "\n=== Running {} test case: {} ===",
        test_mode, test_case.name
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
    let _sourcify_mocks = if with_selectors {
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

    // Add selector resolution flags if requested.
    if with_selectors {
        cmd.arg("--resolve-selectors")
            .arg("--include-args")
            .arg("--include-calldata")
            .env("SOURCIFY_URL", format!("{}/", sourcify_server.url()));
    }

    // Assert the output contains the expected trace.
    let result = cmd.assert().success();

    // When debugging.
    // let stdout = String::from_utf8_lossy(&result.get_output().stdout);
    // let filename = format!("{}-{}.test.log", test_case.name, test_mode);
    // std::fs::write(&filename, stdout.as_ref()).expect("Failed to write test output");
    // println!("Wrote output to {}", filename);

    result.stdout(predicate::str::contains(expected_output));

    println!("✓ {} test case '{}' passed", test_mode, test_case.name);
}

#[test]
fn test_trace_outputs() {
    let test_cases = load_test_fixtures();

    for test_case in test_cases {
        run_trace_test(&test_case, &test_case.expected_trace, "basic", false);
    }
}

#[test]
fn test_full_trace_outputs() {
    let test_cases = load_test_fixtures();

    for test_case in test_cases {
        // Skip test cases that don't have expected_trace_full.
        if let Some(expected_full) = &test_case.expected_trace_full {
            run_trace_test(&test_case, expected_full, "full", true);
        } else {
            println!(
                "⊘ Skipping test case '{}' (no expected_trace_full)",
                test_case.name
            );
        }
    }
}
