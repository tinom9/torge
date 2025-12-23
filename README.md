# Torge

CLI tool to produce Foundry-style traces for EVM transactions through `debug_traceTransaction` RPC requests.

## Installation

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

With argument decoding and calldata:

```bash
torge tx 0x1234...abcd --resolve-selectors --include-args --include-calldata
```
