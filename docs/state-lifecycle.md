# State lifecycle and future persistence

## Current boundaries

`StarknetCheckpoint` is an opaque, consuming, process-local checkpoint of core state. Its exhaustive field inventory includes cached writes, accumulated diffs, classes, archives, transactions, and mempool admission/selection state. Capture does not serialize; restore does not execute transactions. Server checkpoints add journal positions and the simulated fork-origin acceptance boundary. Connections and the snapshot allocator are not part of core state.

The server lifecycle lock coordinates execution, journals, and notification enqueueing. Revert acquires all destination guards and prepares notifications before publishing the retained journal. Its commit contains no suspension or fallible operation. Queue delivery and fallback origin requests run outside the lifecycle lock. An origin fallback captures its acceptance context while the lock is held; an in-flight response can therefore arrive after revert while still describing the earlier read. Revert replaces the derived lookup cache so an older request cannot refill the current cache.

Core execution, checkpoint copying, archive release, file publication, historical subscription setup, and origin reads needed inside execution still contribute to lock hold time. Moving these outside the lock requires either immutable state versions or a prepare/validate/commit design; simply dropping the lock around an existing operation is unsafe. The lifecycle wait/hold histograms expose the cost separately from total RPC latency.

## Dump/load refactor

The current persistent format remains the JSON array of replayable RPC calls. Reading/parsing a file has no side effects. Startup replay and `devnet_load` own invalidation and configured-journal clearing under lifecycle coordination. Importing a separate journal must not delete that source file. Snapshot/revert IDs are neither recorded nor meaningful in persistent dumps.

Native snapshots cannot yet replace this format: Blockifier execution objects, providers, caches, and policy implementations are process objects, not a stable wire schema. A persistent representation must explicitly encode portable state, configuration, classes, transactions, archives, pending execution state, and mempool ordering inputs, then rebuild runtime objects. The checkpoint inventory is the completeness checklist for that work, not a serialization ABI.

The recommended migration is dual-format loading and new-format-only dumping after a versioned state format exists. Detect the legacy top-level array or an explicitly tagged/versioned object; reject unknown formats/versions without resetting state. Decode and validate the whole input, including class preparation, before invalidation. Dispatch legacy input to replay and native input to a prepared checkpoint installation. Keep legacy loading during a documented compatibility period; announce that older binaries cannot read newly produced files before changing default output.

Request/exit exports can encode a complete state image. Block-mode dumping needs a separate decision: rewriting that image after every block costs O(total retained state), unlike the current append journal. A versioned base snapshot plus append-only deltas may be preferable. In-memory checkpoints should continue storing a journal position without assuming a particular on-disk encoding; the journal publisher is the format-specific component. Do not deprecate the existing writer until this cost and crash-recovery model are resolved.

## Future devnet_applyStateDiff

State overrides are mutations, independent of transaction execution. RPC mutation classification is exhaustive so adding a method requires an explicit lifecycle decision. The future endpoint should use exclusive lifecycle access, validate its entire patch before writes, apply through core state APIs, update accumulated diffs and storage-update metadata, and be journaled with canonical explicit values. Its inverse is already expressible through checkpoint/revert; taking an implicit full checkpoint for every override is unnecessary if validation and commit can be separated.

Nonces and storage values in a state diff are absolute assignments, including zero and decreasing nonces. Historic state commits must assign the recorded nonce, and accumulating storage diffs must merge individual slots rather than replace every slot for an address. These invariants now have focused regression tests.

The endpoint still needs explicit semantics for its target (pre-confirmed state versus a newly accepted block), existing executed proposals, stale mempool entries after nonce changes, class replacement, and notifications. Never rewrite accepted historical state in place while keeping its old block hash. A patch to live state must preserve older checkpoints, and a persistent native format must include overrides even when no transaction produced them. The first implementation should validate contracts/keys/nonce ranges, use a cache-aware setter for every field, preserve unaffected slots and classes, and define when overridden values become visible to `latest` and historical queries.

## Regression scenarios

Changes to these boundaries should cover cancellation before revert publication, slow-client isolation, captured acceptance during concurrent revert, empty/nonempty dump restoration, continued execution after nonce/storage changes, and dump/load startup behavior. A persistent-format migration additionally needs compatibility fixtures for every supported version and explicit rejection tests for unknown or malformed formats before any reset.
