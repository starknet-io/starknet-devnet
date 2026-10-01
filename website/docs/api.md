---
sidebar_position: 3
---

# API

## JSON-RPC API

Both Starknet's and Devnet's JSON-RPC API are reachable at `/rpc` and `/`. E.g. if spawning Devnet with default settings, these URLs are functionally equivalent: `http://127.0.0.1:5050/rpc` and `http://127.0.0.1:5050/`. The difference between these two groups of methods is their prefix: `starknet_` (e.g. `starknet_getNonce`) and `devnet_` (e.g. `devnet_mint`).

### Starknet API

Unlike Pythonic Devnet, which also supported Starknet's gateway and feeder gateway API, Devnet in Rust supports [Starknet's JSON-RPC API](https://github.com/starkware-libs/starknet-specs/tree/master/api), including [WebSocket support](#websocket).

Due to how Devnet internally works, the method `starknet_getStorageProof` is not applicable, and thus not supported.

Devnet also provides a proof-related extension method `starknet_proveTransaction`. For supported modes, behavior, and usage examples, check [Transaction proofs and proof modes](./proofs).

Since JSON-RPC v0.6.0, to find out which JSON-RPC version is supported by which Devnet version, check out the [releases page](https://github.com/starknet-io/starknet-devnet/releases).

### Local transaction statuses

Locally submitted transactions execute during submission. Their finality status depends on the [block-generation mode](./blocks):

| Mode                           | Status when submission succeeds | Status after block creation |
| ------------------------------ | ------------------------------- | --------------------------- |
| `transaction` (default)        | `ACCEPTED_ON_L2`                | `ACCEPTED_ON_L2`            |
| `demand` or a numeric interval | `PRE_CONFIRMED`                 | `ACCEPTED_ON_L2`            |

Local transactions never enter `RECEIVED` or `CANDIDATE`. A validation failure returns an error without retaining a transaction. An executed transaction can have execution status `SUCCEEDED` or `REVERTED`; this is independent of its finality status. `ACCEPTED_ON_L1` is available through the explicit [L1 acceptance simulation](./blocks#accepting-blocks-on-l1).

`starknet_getTransactionStatus`, transaction receipts, and transaction-status WebSocket notifications report these executed statuses. An unknown or removed local transaction hash returns `TXN_HASH_NOT_FOUND` (code `29`) from `starknet_getTransactionStatus`. [Snapshot restoration](./dump-load-restart#in-memory-snapshots) can return a retained transaction to its earlier status. In forking mode, responses forwarded to the origin reflect that origin's behavior.

### Devnet API

Devnet has many additional features available via JSON-RPC. The RPC methods are documented throughout the documentation in their corresponding pages, but are also aggregated [here](https://github.com/starknet-io/starknet-devnet/blob/main/website/static/devnet_api.json).

#### Healthcheck

To check if a Devnet instance is alive, send an HTTP request `GET /is_alive`. If alive, Devnet will reply with a `200 OK` and an appropriate message. The optional [embedded UI](./web-ui) and [metrics server](./metrics) also use HTTP endpoints outside the JSON-RPC API.

### WebSocket

The whole [Starknet](#starknet-api) and [Devnet](#devnet-api) JSON-RPC API, including [WebSocket subscription methods](https://github.com/starkware-libs/starknet-specs/blob/v0.10.2/api/starknet_ws_api.json) can be accessed via the WebSocket protocol, using text or binary messages. Devnet listens for new WebSocket connections at `ws://<HOST>:<PORT>/ws` (notice the protocol scheme). E.g. using [`wscat`](https://www.npmjs.com/package/wscat) on the same computer where Devnet is spawned at default host and port:

```
$ wscat -c ws://127.0.0.1:5050/ws
Connected (press CTRL+C to quit)
> { "jsonrpc": "2.0", "id": 0, "method": "starknet_subscribeNewHeads" }
< {"id":0,"result":"2935616350010920547","jsonrpc":"2.0"}
```

#### WebSocket persistence

[Restarting](./dump-load-restart#restarting) and [loading](./dump-load-restart#loading) do not affect Devnet's WebSocket connections, but remove all subscriptions. [Snapshot restoration](./dump-load-restart#in-memory-snapshots) preserves subscriptions and sends notifications for restored state.

## Interacting with Devnet in JavaScript and TypeScript

To spawn Devnet and interact with it using the [Devnet API](#devnet-api), you can use [`starknet-devnet-js`](https://github.com/starknet-io/starknet-devnet-js/). This can be especially useful in achieving [L1-L2 communication](./postman.md).

To interact with Devnet using the [Starknet API](#starknet-api), use [starknet.js](https://starknet-js.com/).

## Config API

Send `devnet_getConfig` to retrieve the resolved startup configuration, including explicit CLI or environment settings and defaults. Use `starknet-devnet --help` to interpret the fields; some field names differ from their CLI options. Runtime changes made by methods such as `devnet_setGasPrice`, `devnet_setTime`, impersonation, or snapshot restoration do not update this response. Use [`devnet_getStatus`](#status-api) for a summary of runtime state and block queries for gas prices currently in effect.

For example, starting with `starknet-devnet --seed 42 --accounts 2 --host 127.0.0.1 --block-generation-on demand --dump-on request --state-archive-capacity full` returns the following result. Class hashes and defaults may change between versions.

```json
{
  "seed": 42,
  "total_accounts": 2,
  "account_contract_class_hash": "0x5b4b537eaa2399e3aa99c4e2e0208ebd6c71bc1467938cd52c798c601e43564",
  "predeployed_accounts_initial_balance": "1000000000000000000000",
  "start_time": null,
  "gas_price_wei": 1000000000,
  "gas_price_fri": 1000000000,
  "data_gas_price_wei": 1000000000,
  "data_gas_price_fri": 1000000000,
  "l2_gas_price_wei": 1000000000,
  "l2_gas_price_fri": 1000000000,
  "chain_id": "SN_SEPOLIA",
  "dump_on": "request",
  "dump_path": null,
  "block_generation_on": "demand",
  "lite_mode": false,
  "proof_mode": "devnet",
  "state_archive": "full",
  "fork_config": {
    "url": null,
    "block_number": null,
    "caching_enabled": null
  },
  "eth_erc20_class_hash": "0xb45dbc3714180381c5680e41931172d67194d77d504413465390e0bef194ec",
  "strk_erc20_class_hash": "0x2e77ee61d4df3d988ee1f42ea5442e913862cc82c2584d212ecda76666498fc",
  "class_size_config": {
    "maximum_contract_class_size": 4089446,
    "maximum_contract_bytecode_size": 81920,
    "maximum_sierra_length": 81920
  },
  "server_config": {
    "host": "127.0.0.1",
    "port": 5050,
    "timeout": 120,
    "restricted_methods": null,
    "ui_enabled": false
  }
}
```

## Status API

To retrieve a summary of the current Devnet state (block count, transaction count, forking info, impersonation state, etc.), send a `JSON-RPC` request with method name `devnet_getStatus`. This method takes no parameters and is useful for dashboards or monitoring tools.

```json
{
  "jsonrpc": "2.0",
  "id": "1",
  "method": "devnet_getStatus"
}
```

The `protocol_version` field reports the JSON-RPC specification version. `block_count` counts retained local accepted blocks, including genesis; `transaction_count` also includes pre-confirmed transactions. `fork_config` is present only when forking and uses the fields `url` and `block`.

For the startup command shown above, the initial result is:

```json
{
  "block_count": 1,
  "transaction_count": 0,
  "pre_confirmed_tx_count": 0,
  "chain_id": "SN_SEPOLIA",
  "protocol_version": "0.10.2",
  "is_forked": false,
  "impersonated_accounts": [],
  "auto_impersonate": false
}
```
