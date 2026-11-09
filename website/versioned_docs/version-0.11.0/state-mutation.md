# Direct state mutation

`devnet_applyStateDiff` atomically changes the effective `pre_confirmed` state without executing a transaction. Use it to prepare test fixtures: write contract storage, set absolute nonces, declare classes, deploy contracts at chosen addresses, or replace contract classes.

## Request and result

The request requires `state_diff`. All categories inside it are optional and default to empty. The optional `classes` array defaults to empty too. An empty request (`{"state_diff": {}}`) succeeds.

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "devnet_applyStateDiff",
  "params": {
    "state_diff": {
      "storage_diffs": [
        {
          "address": "0x123",
          "storage_entries": [{ "key": "0x456", "value": "0x789" }]
        }
      ],
      "nonces": [{ "contract_address": "0x123", "nonce": "0xa" }]
    }
  }
}
```

The example assumes a contract already exists at `0x123`. Storage and nonce targets must exist in effective pre-confirmed state or be deployed by the same request. Nonces are absolute values and can increase, decrease, or be reset to zero. Storage values can also be set to zero.

The result is `{"classes": []}` when no class definitions were supplied. Otherwise, `classes` contains the computed `class_hash` for each supplied definition, in request order, and a `compiled_class_hash` for each Sierra class.

## Classes and deployments

Supply standard Starknet RPC class definitions in `classes`: flattened Sierra classes (with a string ABI) or compressed Cairo 0 classes. Each entry has `contract_class` and optional `class_hash` and `compiled_class_hash`. Devnet computes class hashes and compiles Sierra using its existing compiler and V2 compiled-class hashing; supplied hashes must match. Cairo 0 entries cannot include a compiled-class hash. CASM definitions are not accepted as input.

Supplied definitions automatically declare previously unknown classes. You can optionally include matching `declared_classes` entries (`class_hash`, `compiled_class_hash`) or `deprecated_declared_classes` hashes in `state_diff`. A declaration without a supplied definition must reference an already available class. Supplying an already known matching class succeeds without changing its original declaration block. Classes available through a fork origin can be referenced without supplying their definitions again.

Use `deployed_contracts` entries (`address`, `class_hash`) to deploy at unused, nonzero addresses. This sets the class at the address without running its constructor; initialize required storage in the same request. Use `replaced_classes` entries (`contract_address`, `class_hash`) to replace the class of an existing contract while retaining storage and nonce. Both operations require an available class.

New definitions are readable at `pre_confirmed` immediately and become readable at `latest` when the block is accepted. Historical class data is preserved.

## Ordering and block generation

Successful requests apply in order on top of transactions already executed in the current block. Subsequent submissions, calls, and fee estimation against `pre_confirmed` use the updated state. Existing transaction receipts, traces, events, and statuses retain their original contents. Manual changes create no transaction, receipt, trace, event, or transaction notification.

`latest` and historical accepted state stay unchanged until sealing. Applying a diff never seals a block:

- In `transaction` mode, the next successfully executed transaction seals the accumulated changes. `devnet_createBlock` can seal manual changes alone.
- In `demand` mode, transactions and manual changes accumulate until explicit block creation.
- In interval mode, they accumulate until the timer or explicit block creation, including blocks without transactions.

The accepted block's state update reports final values for overlapping writes. Repeated replacements produce one final replacement. A contract deployed and then replaced within the same block appears as one deployment with its final class hash. A subsequent empty block does not repeat the manual changes.

## Errors and lifecycle compatibility

Invalid requests leave state, accumulated diffs, class registries, and existing transactions unchanged.

| Code     | Meaning                                                                                                        |
| -------- | -------------------------------------------------------------------------------------------------------------- |
| `-32602` | Invalid parameters, duplicate/conflicting entries, supplied-hash mismatch, or unsupported state-diff category. |
| `20`     | A storage, nonce, or replacement target does not exist.                                                        |
| `28`     | A referenced class is unavailable.                                                                             |
| `50`     | A supplied class cannot be converted or compiled.                                                              |
| `-32604` | The method is forbidden by restrictive mode.                                                                   |

Duplicate entries within a category, repeated storage keys, overlapping deployment/replacement targets, incompatible declaration categories, and unknown request fields are rejected. Nonempty `migrated_compiled_classes` is unsupported. Empty migration arrays are accepted.

The method works over HTTP and WebSocket and is restricted by default under `--restrictive-mode`. Snapshots and [dumps](./dump-load-restart#dumping) capture manual changes and supplied classes like any other state. Restart clears them with the rest of Devnet state, and block abortion restores the surviving accepted state.
