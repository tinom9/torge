use assert_cmd::prelude::*;
use mockito::{Matcher, Server, ServerGuard};
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

#[derive(Debug, Deserialize)]
struct ContractsFixture {
    contracts: HashMap<String, String>,
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

fn load_contracts() -> HashMap<String, String> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/contracts.json");
    let content = std::fs::read_to_string(path).expect("Failed to read contracts.json");
    let fixture: ContractsFixture =
        serde_json::from_str(&content).expect("Failed to parse contracts.json");
    fixture.contracts
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

fn setup_sourcify_v2_mock(
    server: &mut Server,
    contracts: &HashMap<String, String>,
) -> Vec<mockito::Mock> {
    contracts
        .iter()
        .map(|(key, name)| {
            let (chain_id, address) = key.split_once(':').expect("key must be chainId:address");
            let endpoint = format!("/v2/contract/{chain_id}/{address}?fields=compilation.name");

            let response = json!({
                "compilation": { "name": name },
                "match": "exact_match",
                "chainId": chain_id,
                "address": address
            });

            server
                .mock("GET", endpoint.as_str())
                .with_status(200)
                .with_header("content-type", "application/json")
                .with_body(response.to_string())
                .create()
        })
        .collect()
}

fn setup_rpc_mock(server: &mut Server, rpc_response: &serde_json::Value) -> mockito::Mock {
    server
        .mock("POST", "/")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(serde_json::to_string(rpc_response).unwrap())
        .create()
}

fn setup_rpc_with_chain_id_mock(
    server: &mut Server,
    rpc_response: &serde_json::Value,
    prestate_response: Option<&serde_json::Value>,
) -> Vec<mockito::Mock> {
    let chain_id_response = json!({ "jsonrpc": "2.0", "id": 1, "result": "0x1" });
    let mut mocks = Vec::new();

    mocks.push(
        server
            .mock("POST", "/")
            .match_body(Matcher::Regex(r#""tracer":"callTracer""#.to_string()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(serde_json::to_string(rpc_response).unwrap())
            .create(),
    );

    if let Some(ps_response) = prestate_response {
        mocks.push(
            server
                .mock("POST", "/")
                .match_body(Matcher::Regex(r#""tracer":"prestateTracer""#.to_string()))
                .with_status(200)
                .with_header("content-type", "application/json")
                .with_body(serde_json::to_string(ps_response).unwrap())
                .create(),
        );
    }

    mocks.push(
        server
            .mock("POST", "/")
            .match_body(Matcher::Regex(r#""method":"eth_chainId""#.to_string()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(chain_id_response.to_string())
            .create(),
    );

    mocks
}

fn load_trace_file(dir: &str, filename: &str) -> String {
    let path = PathBuf::from(dir).join(filename);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("Failed to read trace file: {:?}", path))
}

struct TestEnv {
    rpc_url: String,
    sourcify_url: String,
    sourcify_v2_url: String,
    // Mocks must be dropped before servers (declaration order).
    _mocks: Vec<mockito::Mock>,
    _servers: Vec<ServerGuard>,
}

fn setup_test_env(
    rpc_response: &serde_json::Value,
    prestate_response: Option<&serde_json::Value>,
    mode: TestMode,
) -> TestEnv {
    let mut rpc_server = Server::new();
    let mut sourcify_server = Server::new();
    let mut sourcify_v2_server = Server::new();

    let rpc_url = rpc_server.url();
    let sourcify_url = sourcify_server.url();
    let sourcify_v2_url = sourcify_v2_server.url();

    let mut mocks: Vec<mockito::Mock> = Vec::new();
    if matches!(mode, TestMode::Full) {
        mocks.extend(setup_rpc_with_chain_id_mock(
            &mut rpc_server,
            rpc_response,
            prestate_response,
        ));
        mocks.extend(setup_sourcify_mock(&mut sourcify_server, &load_selectors()));
        mocks.extend(setup_sourcify_v2_mock(
            &mut sourcify_v2_server,
            &load_contracts(),
        ));
    } else {
        mocks.push(setup_rpc_mock(&mut rpc_server, rpc_response));
    }

    TestEnv {
        rpc_url,
        sourcify_url,
        sourcify_v2_url,
        _mocks: mocks,
        _servers: vec![rpc_server, sourcify_server, sourcify_v2_server],
    }
}

fn apply_mode_flags(cmd: &mut Command, mode: TestMode, env: &TestEnv) {
    match mode {
        TestMode::Basic => {}
        TestMode::Logs => {
            cmd.arg("--include-logs");
        }
        TestMode::Full => {
            cmd.arg("--include-logs")
                .arg("--resolve-selectors")
                .arg("--resolve-contracts")
                .arg("--include-args")
                .arg("--include-calldata")
                .arg("--include-storage")
                .env("SOURCIFY_4BYTE_URL", format!("{}/", env.sourcify_url))
                .env("SOURCIFY_SERVER_URL", format!("{}/", env.sourcify_v2_url));
        }
    }
}

fn assert_trace_output(cmd: &mut Command, name: &str, expected_file: &str, expected_output: &str) {
    let output = cmd
        .output()
        .unwrap_or_else(|e| panic!("[{name}] failed to run: {e}"));
    assert!(
        output.status.success(),
        "[{name}] exited with {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(expected_output),
        "[{name}] expected output not found in stdout.\nExpected file: {expected_file}\nStdout:\n{stdout}",
    );
}

// ---------------------------------------------------------------------------
// tx tests
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct TxTestCase {
    name: String,
    tx_hash: String,
    rpc_response: serde_json::Value,
    prestate_response: Option<serde_json::Value>,
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
    let env = setup_test_env(
        &test_case.rpc_response,
        test_case.prestate_response.as_ref(),
        mode,
    );

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("torge"));
    cmd.arg("tx")
        .arg(&test_case.tx_hash)
        .arg("-r")
        .arg(&env.rpc_url)
        .env("TORGE_DISABLE_CACHE", "1");
    apply_mode_flags(&mut cmd, mode, &env);

    assert_trace_output(&mut cmd, &test_case.name, expected_file, &expected_output);
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

// ---------------------------------------------------------------------------
// call tests
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct CallTestCase {
    name: String,
    to: Option<String>,
    data: String,
    from: Option<String>,
    value: Option<String>,
    rpc_response: serde_json::Value,
    prestate_response: Option<serde_json::Value>,
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
    let env = setup_test_env(
        &test_case.rpc_response,
        test_case.prestate_response.as_ref(),
        mode,
    );

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("torge"));
    cmd.arg("call");
    if let Some(to) = &test_case.to {
        cmd.arg(to).arg(&test_case.data);
    } else {
        cmd.arg("--create").arg(&test_case.data);
    }
    cmd.arg("-r")
        .arg(&env.rpc_url)
        .env("TORGE_DISABLE_CACHE", "1");
    if let Some(from) = &test_case.from {
        cmd.arg("--from").arg(from);
    }
    if let Some(value) = &test_case.value {
        cmd.arg("--value").arg(value);
    }
    apply_mode_flags(&mut cmd, mode, &env);

    assert_trace_output(&mut cmd, &test_case.name, expected_file, &expected_output);
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

// ---------------------------------------------------------------------------
// storage-diff edge-case tests
// ---------------------------------------------------------------------------

/// Basic-mode output must never contain storage diffs, even when the fixture
/// carries a `prestate_response`. The flag is absent, so no prestate RPC call
/// should be made.
#[test]
fn test_tx_basic_mode_omits_storage_changes() {
    let fixtures = load_tx_fixtures();
    let tc = fixtures
        .iter()
        .find(|tc| tc.prestate_response.is_some())
        .expect("need at least one fixture with prestate_response");

    let env = setup_test_env(&tc.rpc_response, None, TestMode::Basic);

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("torge"));
    cmd.arg("tx")
        .arg(&tc.tx_hash)
        .arg("-r")
        .arg(&env.rpc_url)
        .env("TORGE_DISABLE_CACHE", "1");

    let output = cmd.output().expect("failed to run");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("Storage changes:"),
        "basic mode must not include storage diffs"
    );
}

/// When the prestateTracer RPC call fails, the tool should still succeed and
/// print the call trace. The storage diff warning goes to stderr.
#[test]
fn test_tx_prestate_rpc_failure_degrades_gracefully() {
    let fixtures = load_tx_fixtures();
    let tc = fixtures
        .iter()
        .find(|tc| tc.prestate_response.is_some())
        .expect("need at least one fixture with prestate_response");

    let mut rpc_server = Server::new();
    let mut sourcify_server = Server::new();
    let mut sourcify_v2_server = Server::new();

    let rpc_url = rpc_server.url();
    let sourcify_url = sourcify_server.url();
    let sourcify_v2_url = sourcify_v2_server.url();

    let chain_id_response = json!({ "jsonrpc": "2.0", "id": 1, "result": "0x1" });

    let calltrace_mock = rpc_server
        .mock("POST", "/")
        .match_body(Matcher::Regex(r#""tracer":"callTracer""#.to_string()))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(serde_json::to_string(&tc.rpc_response).unwrap())
        .create();

    let prestate_mock = rpc_server
        .mock("POST", "/")
        .match_body(Matcher::Regex(r#""tracer":"prestateTracer""#.to_string()))
        .with_status(500)
        .with_body("internal server error")
        .create();

    let chain_id_mock = rpc_server
        .mock("POST", "/")
        .match_body(Matcher::Regex(r#""method":"eth_chainId""#.to_string()))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(chain_id_response.to_string())
        .create();

    let selector_mocks = setup_sourcify_mock(&mut sourcify_server, &load_selectors());
    let contract_mocks = setup_sourcify_v2_mock(&mut sourcify_v2_server, &load_contracts());

    let _mocks = (
        calltrace_mock,
        prestate_mock,
        chain_id_mock,
        selector_mocks,
        contract_mocks,
    );
    let _servers = vec![rpc_server, sourcify_server, sourcify_v2_server];

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("torge"));
    cmd.arg("tx")
        .arg(&tc.tx_hash)
        .arg("-r")
        .arg(&rpc_url)
        .arg("--include-logs")
        .arg("--resolve-selectors")
        .arg("--resolve-contracts")
        .arg("--include-args")
        .arg("--include-calldata")
        .arg("--include-storage")
        .env("TORGE_DISABLE_CACHE", "1")
        .env("SOURCIFY_4BYTE_URL", format!("{sourcify_url}/"))
        .env("SOURCIFY_SERVER_URL", format!("{sourcify_v2_url}/"));

    let output = cmd.output().expect("failed to run");
    assert!(
        output.status.success(),
        "should succeed despite prestate failure: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stdout.contains("Storage changes:"),
        "storage diffs should not appear when prestate RPC fails"
    );
    assert!(
        stderr.contains("storage diff unavailable"),
        "stderr should contain degradation warning, got: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// clean tests
// ---------------------------------------------------------------------------

#[test]
fn test_clean_only_unknown() {
    let tmp = TempDir::new().unwrap();
    let torge_dir = tmp.path().join("torge");
    std::fs::create_dir_all(&torge_dir).unwrap();

    let selectors_path = torge_dir.join("selectors.json");
    let selectors_cache = json!({
        "selectors": {
            "0xa9059cbb": "transfer(address,uint256)",
            "0x70a08231": "balanceOf(address)",
            "0xdeadbeef": "<UNKNOWN>",
            "0xcafebabe": "<UNKNOWN>"
        }
    });
    std::fs::write(
        &selectors_path,
        serde_json::to_string_pretty(&selectors_cache).unwrap(),
    )
    .unwrap();

    let contracts_path = torge_dir.join("contracts.json");
    let contracts_cache = json!({
        "contracts": {
            "1:0xdac17f958d2ee523a2206206994597c13d831ec7": "TetherToken",
            "1:0x0000000000000000000000000000000000c0ffee": "<UNKNOWN>"
        }
    });
    std::fs::write(
        &contracts_path,
        serde_json::to_string_pretty(&contracts_cache).unwrap(),
    )
    .unwrap();

    let binary_path = assert_cmd::cargo::cargo_bin!("torge");
    let mut cmd = Command::new(binary_path);
    cmd.arg("clean")
        .arg("--only-unknown")
        .env("XDG_CACHE_HOME", tmp.path());

    cmd.assert()
        .success()
        .stdout(predicate::str::contains(
            "selectors: removed 2 unknown, kept 2",
        ))
        .stdout(predicate::str::contains(
            "contracts: removed 1 unknown, kept 1",
        ));

    let remaining: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&selectors_path).unwrap()).unwrap();
    let selectors = remaining["selectors"].as_object().unwrap();
    assert_eq!(selectors.len(), 2);
    assert!(selectors.contains_key("0xa9059cbb"));
    assert!(selectors.contains_key("0x70a08231"));

    let remaining: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&contracts_path).unwrap()).unwrap();
    let contracts = remaining["contracts"].as_object().unwrap();
    assert_eq!(contracts.len(), 1);
    assert!(contracts.contains_key("1:0xdac17f958d2ee523a2206206994597c13d831ec7"));
}

#[test]
fn test_clean_removes_entire_cache() {
    let tmp = TempDir::new().unwrap();
    let torge_dir = tmp.path().join("torge");
    std::fs::create_dir_all(&torge_dir).unwrap();

    let selectors_path = torge_dir.join("selectors.json");
    std::fs::write(
        &selectors_path,
        serde_json::to_string_pretty(&json!({
            "selectors": { "0xa9059cbb": "transfer(address,uint256)" }
        }))
        .unwrap(),
    )
    .unwrap();

    let contracts_path = torge_dir.join("contracts.json");
    std::fs::write(
        &contracts_path,
        serde_json::to_string_pretty(&json!({
            "contracts": { "1:0xdead": "SomeContract" }
        }))
        .unwrap(),
    )
    .unwrap();

    let binary_path = assert_cmd::cargo::cargo_bin!("torge");
    let mut cmd = Command::new(binary_path);
    cmd.arg("clean").env("XDG_CACHE_HOME", tmp.path());

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("selectors: cache cleared"))
        .stdout(predicate::str::contains("contracts: cache cleared"));

    assert!(!selectors_path.exists());
    assert!(!contracts_path.exists());
}
