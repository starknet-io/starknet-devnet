# Web UI explorer

Devnet ships with a web UI that lets you explore blocks, transactions, accounts, and configuration from a browser instead of hand-crafting JSON-RPC requests. The UI is embedded into the Devnet binary and served on the same port as the JSON-RPC API, so no extra setup is needed once it is enabled.

The UI is **opt-in** — it is not served unless you explicitly turn it on. When enabled, it is reachable at `/ui` (and `/ui/`) on the configured Devnet host and port.

## Enabling the Web UI

Start Devnet with the `--ui` flag:

```bash
$ starknet-devnet --ui
```

Or set the equivalent environment variable:

```bash
$ UI=true starknet-devnet
```

E.g. on the default host and port, the UI is then available at:

```
http://127.0.0.1:5050/ui
```

### Docker

Because the UI is served on the same port as the JSON-RPC API, no extra port publishing is required — the existing `-p 5050:5050` mapping is enough:

```bash
$ docker run -p 5050:5050 starknetfoundation/starknet-devnet-rs --ui
```

Then open `http://127.0.0.1:5050/ui` in your browser.

## Features

The UI is organised around the same data Devnet exposes via JSON-RPC, so each screen maps onto a documented method in the [API reference](./api.md):

- **Dashboard** — at-a-glance counters (block count, transaction count, pre-confirmed transactions, chain id, RPC version), forking indicator, and impersonation state.
- **Blocks** — list of accepted blocks and a link to the live pre-confirmed block; click through to see block contents and state diffs, including direct state changes without transactions.
- **Transactions** — transaction detail pages showing status, receipt, and execution trace where available.
- **Accounts** — view [predeployed accounts](./predeployed.md) along with their addresses and balances.
- **Control panel** — fire Devnet-specific actions (mint tokens, create or abort blocks, set time, set L1/L2/data gas prices, snapshot/revert, dump/load/restart state, impersonate accounts, postman flush/load, etc.) without writing JSON-RPC by hand. Apply State Diff accepts JSON parameters and supplied class definitions, displays computed hashes, and leaves changes pre-confirmed until sealing. Dump responses can be copied as JSON and loaded as inline events or from a server file.
- **Config** — startup configuration with runtime status and a comparison against current block gas prices.
- **Connection settings** — the embedded UI defaults to its own server's RPC endpoint, including nondefault ports; point it at another Devnet when needed. The sidebar displays the connected Devnet's version, or “Version unavailable” for older instances that do not report it.

Snapshots are process-local. Successful revert consumes the selected snapshot and all newer snapshots; load and restart invalidate all snapshots. These actions refresh cached explorer views so discarded blocks, transactions, and balances are not displayed as current state. Snapshot/revert and Apply State Diff remain restricted by default in restrictive mode; adding their controls does not make them required for the explorer to connect.

:::tip

The UI is a regular HTTP client — it talks to Devnet through the standard JSON-RPC API at the same origin. You can use it together with `starknet-devnet-js` and other tooling; there is no separate port or protocol to configure.

:::

## Disabling the Web UI

By default, the UI is **not** served. Leaving `--ui` unset (or unsetting the `UI` env var) disables the UI routes. JSON-RPC and the healthcheck remain available; the optional metrics server is configured separately.
