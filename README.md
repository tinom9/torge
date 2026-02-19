# Torge

[![Crates.io](https://img.shields.io/crates/v/torge.svg)](https://crates.io/crates/torge)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

CLI tool to produce Foundry-style traces for EVM transactions and calls through `debug_traceTransaction` and `debug_traceCall` RPC requests.

## Installation

From [crates.io](https://crates.io/crates/torge):

```bash
cargo install torge
```

From source:

```bash
cargo install --path .
```

## Usage

### `torge tx` — Trace a transaction

```bash
torge tx <TX_HASH> [OPTIONS]
```

Basic usage:

```bash
torge tx 0x1234...abcd --rpc-url http://localhost:8545
```

With Foundry alias:

```bash
torge tx 0x1234...abcd --rpc-url ethereum
```

With selector resolution:

```bash
torge tx 0x1234...abcd --rpc-url http://localhost:8545 --resolve-selectors
```

With argument decoding, calldata, and events:

```bash
torge tx 0x1234...abcd --resolve-selectors --include-args --include-calldata --include-logs
```

### `torge call` — Simulate a call

```bash
torge call <TO> <DATA> [OPTIONS]
torge call --create <DATA> [OPTIONS]
```

Simulate a call against a contract:

```bash
torge call 0xdead...beef 0xa9059cbb... --rpc-url http://localhost:8545
```

With sender, value, and gas limit:

```bash
torge call 0xdead...beef 0xa9059cbb... --from 0xcafe...1234 --value 1ether --gas-limit 1000000
```

At a specific block:

```bash
torge call 0xdead...beef 0xa9059cbb... --block 18000000
```

Simulate a contract creation:

```bash
torge call --create 0x6080604052... --rpc-url http://localhost:8545
```

### `torge clean` — Manage selector cache

```bash
torge clean [OPTIONS]
```

Clear the entire selector cache:

```bash
torge clean
```

Remove only unresolved selectors (retry on next lookup):

```bash
torge clean --only-unknown
```
