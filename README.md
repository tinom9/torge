# Torge

[![Crates.io](https://img.shields.io/crates/v/torge.svg)](https://crates.io/crates/torge)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

CLI tool to produce Foundry-style traces for EVM transactions through `debug_traceTransaction` RPC requests.

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

### Trace a transaction

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
