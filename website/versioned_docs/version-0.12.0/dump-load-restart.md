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

Each live snapshot retains a full logical copy of Devnet-owned state. Creation cost and memory use grow with retained state and are highest with full state archives. Reverting can also release a large discarded state graph.

Capture takes O(total retained state) time and additional memory; there is no configured snapshot-count limit. Capture and restore exclude concurrent state operations, so large archives can pause RPCs and delay interval sealing. Snapshot IDs accept `0x`-prefixed hexadecimal digits, including uppercase digits and leading zeros, within the `u64` range. Zero and unknown IDs return `false`; malformed or overflowing IDs return invalid params. Both methods are available over HTTP and WebSocket and restricted by default in restrictive mode.

Timestamp controls are restored exactly, but wall-clock time continues to advance. A read already forwarded to a fork origin may finish after revert using the acceptance boundary captured before revert. A failed load leaves snapshots intact, while a successful load or restart invalidates them. Dumps do not contain snapshots, and reverting does not change dump files.

External L1 state, live connections, interval scheduling, and global fork caches are not checkpointed. Messaging cursors are restored, so events still present on an external L1 may become visible to Devnet again. Existing WebSocket subscriptions remain connected and receive a reorg notification when confirmed blocks are displaced, followed by notifications for each restored block in order, including its header, accepted transactions, receipts, statuses, and events.

Transaction-status subscriptions also receive updates when revert changes a retained transaction's L1 acceptance status, even if its block hash stays unchanged. Restored pre-confirmed transactions produce status updates when their status changes or they were absent before revert; unchanged statuses are not sent again. Fork-origin event queries release lifecycle access during origin requests, allowing snapshot, revert, and other state-changing RPCs to continue while the origin responds.

## Dumping

A dump preserves a Devnet instance: its configuration and everything that happened after genesis. This includes blocks, transactions with their receipts and traces, state changes (including pre-confirmed ones and those made with [`devnet_applyStateDiff`](./state-mutation)), time, gas and impersonation settings, and [L1-L2 messaging](./postman) state. Loading restores it exactly, including block hashes and timestamps, without re-executing transactions.

A dump does not contain [snapshots](#in-memory-snapshots), WebSocket subscriptions, metrics, the state of an external L1 node, or the forking origin's chain.

### Dumping on request

`devnet_dump` returns the dump as a JSON object. It takes no parameters, needs no startup option and writes no files:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "devnet_dump"
}
```

### Dumping on exit

To dump when Devnet is stopped with Ctrl+C (SIGINT), specify a file:

```
$ starknet-devnet --dump-on exit --dump-path <PATH>
```

The file is replaced only once the new dump has been written completely. If writing fails, the previous file is kept and Devnet exits with an error. Termination that Devnet cannot handle, such as SIGKILL or a crash, produces no dump.

:::note

`--dump-on block` and `--dump-on request` were removed: request a dump with `devnet_dump` whenever needed. The `path` and `inline` parameters of `devnet_dump` were removed too.

:::

:::caution

A dump of a forked instance contains the forking URL, which may include an API key. If you loaded the L1 messaging contract with a custom `deployer_account_private_key`, the dump contains that key as well.

:::

## Loading

### Loading on startup

```
$ starknet-devnet --load-path <PATH>
```

Without `--load-path`, Devnet loads from `--dump-path` if a file exists there, so `--dump-on exit --dump-path <PATH>` continues where the previous run stopped.

A loaded dump applies its own configuration, such as the predeployed accounts, chain ID, gas prices, block generation mode and state archive capacity. Options you specify must match the dump; otherwise Devnet does not start and names the conflicting options. Server options such as `--host`, `--port` or `--restrictive-mode` are not part of a dump and apply as usual.

A dump of a forked instance reconnects to the forking origin it was created with. Specify `--fork-network` to use a different URL, e.g. another endpoint of the same network. Either way, the origin's block at the fork point must match the dump.

### Loading on request

`devnet_load` replaces the current state with the dump returned by `devnet_dump`:

```
JSON-RPC
{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "devnet_load",
    "params": {
        "dump": <the result of devnet_dump>
    }
}
```

The result is an empty object. A dump whose block generation mode or forking origin differs from the running instance cannot be loaded on request; load it on startup instead. The `path` and `events` parameters were removed; Devnet loads files only on startup.

Both HTTP and WebSocket accept dumps of any size. Very large dumps can take longer than the HTTP request timeout, which you can raise with `--timeout`.

### Effects of loading

Loading invalidates all [snapshots](#in-memory-snapshots) without reusing their identifiers. It does not affect WebSocket connections, but removes all WebSocket [subscriptions](./api#websocket). A load that fails leaves the state, snapshots and subscriptions unchanged.

Loading does not contact the L1 node. The L1 messaging configuration and Devnet's position in the L1 message history are restored, so the next [flush](./postman#flush) continues where the dumped instance left off.

### Compatibility

A dump loads only into a Devnet that produces the same genesis (predeployed accounts, fee tokens, predeclared classes) for the dumped configuration, which in practice means the same Devnet release. Other dumps are rejected with an error.

### Legacy dumps

Earlier Devnet versions dumped a list of replayable requests. During a transition period, these legacy dumps can still be loaded, on startup or with `devnet_load` by passing the list as `dump`. Loading a legacy dump re-executes its requests, so block timestamps differ from the original run, and requires the same options, such as `--account-class` and `--block-generation-on`, as when it was dumped. Legacy dumps that contain `devnet_dump` or `devnet_load` requests are rejected. Devnet no longer produces legacy dumps.

To convert a legacy dump, specify the options it was dumped with:

```
$ starknet-devnet --load-path <LEGACY_PATH> --convert-legacy-dump <OUTPUT_PATH> [options]
```

Devnet writes the converted dump to `<OUTPUT_PATH>` and exits without starting the server.

## Restarting

Devnet can be restarted by making a `JSON-RPC` request with method name `devnet_restart`. All deployed contracts (including predeployed), blocks and storage updates will be restarted to the original state, without the transactions and requests that may have been loaded from a dump file on startup.

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
  --load-path /path/to/dumpdir/mydump
```

To dump to `/path/to/dumpdir/mydump` on Devnet shutdown, run:

```
docker run \
  -p 127.0.0.1:5050:5050 \
  --mount type=bind,source=/path/to/dumpdir,target=/path/to/dumpdir \
  starknetfoundation/starknet-devnet-rs \
  --dump-on exit --dump-path /path/to/dumpdir/mydump
```
