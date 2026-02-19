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
struct SelectorsFixture {
    selectors: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy)]
enum TestMode {
    Basic,
    Logs,
    Full,
}

fn load_selectors() -> HashMap<String, String> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/selectors.json");
    let content = std::fs::read_to_string(path).expect("Failed to read selectors.json");
    let fixture: SelectorsFixture =
        serde_json::from_str(&content).expect("Failed to parse selectors.json");
    fixture.selectors
}

fn setup_sourcify_mock(
    server: &mut Server,
    selectors: &HashMap<String, String>,
) -> Vec<mockito::Mock> {
    selectors
        .iter()
        .flat_map(|(selector, signature)| {
            let is_event = selector.len() == 66;

            let response = if signature == "<UNKNOWN>" {
                if is_event {
                    json!({ "ok": true, "result": { "event": {} } })
                } else {
                    json!({ "ok": true, "result": { "function": {} } })
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

            let kind = if is_event { "event" } else { "function" };
            let endpoint = format!("/signature-database/v1/lookup?{kind}={selector}&filter=false");

            vec![server
                .mock("GET", endpoint.as_str())
                .with_status(200)
                .with_header("content-type", "application/json")
                .with_body(response.to_string())
                .create()]
        })
        .collect()
}

fn load_trace_file(dir: &str, filename: &str) -> String {
    let path = PathBuf::from(dir).join(filename);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("Failed to read trace file: {:?}", path))
}

#[derive(Debug, Deserialize)]
struct TxTestCase {
    name: String,
    tx_hash: String,
    rpc_response: serde_json::Value,
    expected_trace: String,
    expected_trace_logs: Option<String>,
    expected_trace_full: Option<String>,
}

fn load_tx_fixtures() -> Vec<TxTestCase> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/tx/main.json");
    let content = std::fs::read_to_string(path).expect("Failed to read tx/main.json");
    serde_json::from_str(&content).expect("Failed to parse tx/main.json")
}

fn run_tx_test(test_case: &TxTestCase, expected_file: &str, mode: TestMode) {
    let traces_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/tx/traces");
    let expected_output = load_trace_file(traces_dir, expected_file);

    let mut rpc_server = Server::new();
    let _rpc_mock = rpc_server
        .mock("POST", "/")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(serde_json::to_string(&test_case.rpc_response).unwrap())
        .create();

    let mut sourcify_server = Server::new();
    let _sourcify_mocks = matches!(mode, TestMode::Full).then(|| {
        let selectors = load_selectors();
        setup_sourcify_mock(&mut sourcify_server, &selectors)
    });

    let binary_path = assert_cmd::cargo::cargo_bin!("torge");
    let mut cmd = Command::new(binary_path);

    cmd.arg("tx")
        .arg(&test_case.tx_hash)
        .arg("-r")
        .arg(rpc_server.url())
        .env("TORGE_DISABLE_CACHE", "1");

    match mode {
        TestMode::Basic => {}
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

    let output = cmd
        .output()
        .unwrap_or_else(|e| panic!("[{}] failed to run: {e}", test_case.name));
    assert!(
        output.status.success(),
        "[{}] exited with {}: {}",
        test_case.name,
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(&expected_output),
        "[{}] expected output not found in stdout.\nExpected file: {expected_file}\nStdout:\n{stdout}",
        test_case.name
    );
}

#[test]
fn test_tx_trace_outputs() {
    for tc in load_tx_fixtures() {
        run_tx_test(&tc, &tc.expected_trace, TestMode::Basic);
    }
}

#[test]
fn test_tx_logs_trace_outputs() {
    for tc in load_tx_fixtures() {
        if let Some(f) = &tc.expected_trace_logs {
            run_tx_test(&tc, f, TestMode::Logs);
        }
    }
}

#[test]
fn test_tx_full_trace_outputs() {
    for tc in load_tx_fixtures() {
        if let Some(f) = &tc.expected_trace_full {
            run_tx_test(&tc, f, TestMode::Full);
        }
    }
}

#[test]
fn test_clean_only_unknown() {
    let tmp = TempDir::new().unwrap();
    let cache_path = tmp.path().join("torge").join("selectors.json");
    std::fs::create_dir_all(cache_path.parent().unwrap()).unwrap();

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

#[derive(Debug, Deserialize)]
struct CallTestCase {
    name: String,
    to: Option<String>,
    data: String,
    from: Option<String>,
    value: Option<String>,
    rpc_response: serde_json::Value,
    expected_trace: String,
    expected_trace_logs: Option<String>,
    expected_trace_full: Option<String>,
}

fn load_call_fixtures() -> Vec<CallTestCase> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/call/main.json");
    let content = std::fs::read_to_string(path).expect("Failed to read call/main.json");
    serde_json::from_str(&content).expect("Failed to parse call/main.json")
}

fn run_call_test(test_case: &CallTestCase, expected_file: &str, mode: TestMode) {
    let traces_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/call/traces");
    let expected_output = load_trace_file(traces_dir, expected_file);

    let mut rpc_server = Server::new();
    let _rpc_mock = rpc_server
        .mock("POST", "/")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(serde_json::to_string(&test_case.rpc_response).unwrap())
        .create();

    let mut sourcify_server = Server::new();
    let _sourcify_mocks = matches!(mode, TestMode::Full).then(|| {
        let selectors = load_selectors();
        setup_sourcify_mock(&mut sourcify_server, &selectors)
    });

    let binary_path = assert_cmd::cargo::cargo_bin!("torge");
    let mut cmd = Command::new(binary_path);

    cmd.arg("call");

    // Positional args: [TO] <DATA>.
    if let Some(to) = &test_case.to {
        cmd.arg(to).arg(&test_case.data);
    } else {
        cmd.arg("--create").arg(&test_case.data);
    }

    cmd.arg("-r")
        .arg(rpc_server.url())
        .env("TORGE_DISABLE_CACHE", "1");
    if let Some(from) = &test_case.from {
        cmd.arg("--from").arg(from);
    }
    if let Some(value) = &test_case.value {
        cmd.arg("--value").arg(value);
    }

    match mode {
        TestMode::Basic => {}
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

    let output = cmd
        .output()
        .unwrap_or_else(|e| panic!("[{}] failed to run: {e}", test_case.name));
    assert!(
        output.status.success(),
        "[{}] exited with {}: {}",
        test_case.name,
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(&expected_output),
        "[{}] expected output not found in stdout.\nExpected file: {expected_file}\nStdout:\n{stdout}",
        test_case.name
    );
}

#[test]
fn test_call_trace_outputs() {
    for tc in load_call_fixtures() {
        run_call_test(&tc, &tc.expected_trace, TestMode::Basic);
    }
}

#[test]
fn test_call_logs_trace_outputs() {
    for tc in load_call_fixtures() {
        if let Some(f) = &tc.expected_trace_logs {
            run_call_test(&tc, f, TestMode::Logs);
        }
    }
}

#[test]
fn test_call_full_trace_outputs() {
    for tc in load_call_fixtures() {
        if let Some(f) = &tc.expected_trace_full {
            run_call_test(&tc, f, TestMode::Full);
        }
    }
}
