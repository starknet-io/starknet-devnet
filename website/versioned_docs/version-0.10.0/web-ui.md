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
- **Blocks** — list of accepted and pre-confirmed blocks with their hashes and transaction counts; click through to see full block contents.
- **Transactions** — transaction detail pages showing status, receipt, and execution trace where available.
- **Accounts** — view [predeployed accounts](./predeployed.md) along with their addresses and balances.
- **Control panel** — fire Devnet-specific actions (mint tokens, create or abort blocks, set time, set gas price, dump/load/restart state, impersonate accounts, postman flush/load, etc.) without writing JSON-RPC by hand.
- **Config** — current Devnet configuration with the runtime status overlaid for convenience.
- **Connection settings** — point the UI at a different Devnet endpoint (handy when Devnet is running in a different container or on a remote host).

:::tip

The UI is a regular HTTP client — it talks to Devnet through the standard JSON-RPC API at the same origin. You can use it together with `starknet-devnet-js` and other tooling; there is no separate port or protocol to configure.

:::

## Disabling the Web UI

By default, the UI is **not** served. Leaving `--ui` unset (or unsetting the `UI` env var) keeps the server behaviour identical to previous Devnet versions — only the JSON-RPC API is exposed.
