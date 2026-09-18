# Dump, load, restart

## In-memory snapshots

Use snapshots to restore Devnet directly between tests without replaying transactions. Snapshots are process-local and remain in memory until they are reverted, invalidated, or the process exits.

Create a snapshot:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "devnet_snapshot",
  "params": []
}
```

The result is a monotonically increasing hexadecimal identifier such as `"0x1"`. Restore it with:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "devnet_revert",
  "params": { "snapshot_id": "0x1" }
}
```

The result is `true` when the checkpoint was restored and `false` when it is unknown, consumed, or invalidated. Reverting consumes the target and every newer snapshot while preserving older snapshots. `devnet_restart` and `devnet_load` invalidate all snapshots without reusing their identifiers.

For fixture reuse, create the fixture once, snapshot it, run one test, revert, and immediately create a replacement snapshot for the next test because a successful revert consumes its identifier.

For example, assuming `rpc(method, params)` returns the JSON-RPC result, call `resetFixture()` between tests:

```javascript
await setupFixture();
let snapshotId = await rpc('devnet_snapshot', []);

async function resetFixture() {
  const restored = await rpc('devnet_revert', { snapshot_id: snapshotId });
  if (!restored) throw new Error('Fixture snapshot is no longer available');
  snapshotId = await rpc('devnet_snapshot', []);
}
```

Each live snapshot retains a full logical copy of Devnet-owned state. Creation cost and memory use grow with retained state and are highest with full state archives. Reverting can also rewrite the configured block-mode dump file and release a large discarded state graph.

Capture takes O(total retained state) time and additional memory; snapshots are unlimited and v1 imposes no performance ceiling. Capture and restore exclude concurrent state operations, so large archives can pause RPCs and delay interval sealing. Snapshot IDs accept `0x`-prefixed hexadecimal digits, including uppercase digits and leading zeros, within the `u64` range. Zero and unknown IDs return `false`; malformed or overflowing IDs return invalid params. Both methods are available over HTTP and WebSocket and restricted by default in restrictive mode.

Timestamp controls are restored exactly, but wall-clock time continues to advance. A read already forwarded to a fork origin may finish after revert using the acceptance boundary captured before revert. File-read and parse failures during load preserve snapshots; once destructive load/restart work begins, snapshots are invalidated even if later work fails. Request/exit journals are truncated by revert; separately exported files remain unchanged. Dump files still use the replay journal format and cannot store process-local snapshot IDs.

External L1 state, live connections, interval scheduling, and global fork caches are not checkpointed. Messaging cursors are restored, so events still present on an external L1 may become visible to Devnet again. Existing WebSocket subscriptions remain connected and receive a reorg notification when confirmed blocks are displaced.

## Dumping

To preserve your Devnet instance for future use, these are the options:

- Dumping on exit (handles Ctrl+C, i.e. SIGINT; doesn't handle SIGKILL):

```
$ starknet-devnet --dump-on exit --dump-path <PATH>
```

- Dumping after each block:

```
$ starknet-devnet --dump-on block --dump-path <PATH>
```

- Dumping on request, which requires providing `--dump-on request` on startup. You can also dump on request if you specified any of the other `--dump-on` modes.

### Dumping on request

You can request dumping via JSON-RPC. An optional file path can be provided in the request or on startup via `--dump-path <FILE>` (the JSON-RPC request parameter takes precedence). The dumped events are always included in the response, including when they are also written to a file.

By default, a request without a `path` uses the path supplied through `--dump-path`, if one was configured. Set `inline` to `true` to ignore that startup path and return the events without writing to it. A `path` supplied in the same request is still written to, even when `inline` is `true`.

```
$ starknet-devnet --dump-on <MODE> [--dump-path <FILE>]
```

- Use the startup path if configured; otherwise return the events without writing a file:

```
JSON-RPC
{
    "jsonrpc": "2.0",
    "id": "1",
    "method": "devnet_dump"
}
```

- Write to a custom path and return the events:

```
JSON-RPC
{
    "jsonrpc": "2.0",
    "id": "1",
    "method": "devnet_dump",
    "params": {
        // optional; defaults to the path specified via CLI if defined
        "path": <PATH>
    }
}
```

- Return the events without writing to the startup path:

```
JSON-RPC
{
    "jsonrpc": "2.0",
    "id": "1",
    "method": "devnet_dump",
    "params": {
        "inline": true
    }
}
```

## Loading

To load a preserved Devnet instance, the options are:

- Loading on startup (note the argument name is not `--load-path` as it was in Devnet-py):

```
$ starknet-devnet --dump-path <PATH>
```

- Loading on request, which replaces the current state by re-executing events. Provide either a dump file path or dumped events directly in the request body.

To load from a file:

```
JSON-RPC
{
    "jsonrpc": "2.0",
    "id": "1",
    "method": "devnet_load",
    "params": {
        "path": <PATH>
    }
}
```

To load events directly:

```
JSON-RPC
{
    "jsonrpc": "2.0",
    "id": "1",
    "method": "devnet_load",
    "params": {
        "events": [
            // events returned by devnet_dump
        ]
    }
}
```

The `path` and `events` parameters are mutually exclusive. Passing both, or neither, results in an invalid-params error.

### Loading disclaimer

Currently, dumping produces a list of reproducible Devnet actions (state-changing requests and transactions). Conversely, loading is implemented as the re-execution of transactions from a dump. This means that timestamps of `StarknetBlock` will be different on each load. This is due to the nature of Devnet's dependencies, which prevent Devnet's state from being serialized.

In [mempool mode](./mempool), a policy-driven processing action is dumped as the exact ordered transaction hashes that were selected. Mempool configuration changes, removal, clearing, strict sealing, and proposal abortion are also recorded. This makes loading deterministic even for seeded-random ordering and avoids depending on wall-clock timing or policy defaults.

Dumping and loading are not guaranteed to work across versions. I.e. if you dumped one version of Devnet, do not expect it to be loadable with a different version.

If you dumped a Devnet utilizing one class for account predeployment (e.g. `--account-class cairo0`), you should use the same option when loading. The same applies to the block-generation and mempool configuration used by the dumped Devnet.

Loading does not affect WebSocket connections, but removes all WebSocket [subscriptions](./api#websocket).

## Restarting

Devnet can be restarted by making a `JSON-RPC` request with method name `devnet_restart`. All deployed contracts (including predeployed), blocks and storage updates will be restarted to the original state, without the transactions and requests that may have been loaded from a dump file on startup. Restarting also clears all received and candidate transactions and the open pre-confirmed proposal.

Restarting does not affect WebSocket connections, but removes all WebSocket [subscriptions](./api#websocket).

### Restarting and L1-L2 messaging

If you're doing [L1-L2 message exchange](./postman), restarting will by default not affect Devnet's connection with L1 nor the L1->L2 message queue. The effect that L1-L2 messages may have had on Devnet before restarting shall be reverted, including any L2 contracts used for messaging. Also, calling [`flush`](./postman#flush) will not have new messages to read until they are actually sent. If you wish to re-process the already-seen L1->L2 messages when you restart, make them accessible again by setting the `restart_l1_to_l2_messaging` parameter shown below. If you set this flag:

- you will need to [reload the L1-side messaging contract](./postman#load)
- the L1->L2 messages won't be restarted in the sense of being deleted, but access to them shall be regained via [`flush`](./postman#flush)
- the L2->L1 message queue is restarted regardless of the flag

```
JSON-RPC
{
    "jsonrpc": "2.0",
    "id": "1",
    "method": "devnet_restart",
    "params": {
        // optional parameter, defaults to false
        "restart_l1_to_l2_messaging": true | false
    }
}
```

## Docker

To enable dumping and loading with dockerized Devnet, you must bind the container path to the path on your host machine.

This example:

- Relies on [Docker bind mount](https://docs.docker.com/storage/bind-mounts/); try [Docker volume](https://docs.docker.com/storage/volumes/) instead.
- Assumes that `/path/to/dumpdir` exists. If unsure, use absolute paths.
- Assumes you are listening on `127.0.0.1:5050`.

If there is `mydump` inside `/path/to/dumpdir`, you can load it with:

```
docker run \
  -p 127.0.0.1:5050:5050 \
  --mount type=bind,source=/path/to/dumpdir,target=/path/to/dumpdir \
  starknetfoundation/starknet-devnet-rs \
  --dump-path /path/to/dumpdir/mydump
```

To dump to `/path/to/dumpdir/mydump` on Devnet shutdown, run:

```
docker run \
  -p 127.0.0.1:5050:5050 \
  --mount type=bind,source=/path/to/dumpdir,target=/path/to/dumpdir \
  starknetfoundation/starknet-devnet-rs \
  --dump-on exit --dump-path /path/to/dumpdir/mydump
```
