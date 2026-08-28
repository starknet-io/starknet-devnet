# Mempool

Manual mempool mode separates transaction admission, preconfirmation, and block sealing. Start it with `starknet-devnet --block-generation-on mempool`. Standard add-transaction methods return after stateless preparation and admission, while the transaction remains `RECEIVED` and has not yet affected Devnet state.

## Transaction lifecycle

- `RECEIVED`: admitted to the pool and waiting to become nonce-eligible and selected.
- `CANDIDATE`: selected for processing but not yet published in the live pre-confirmed block.
- `PRE_CONFIRMED`: executed and appended to the live proposal. Later transactions execute against its speculative state.
- `ACCEPTED_ON_L2`: permanently included when the proposal is sealed.

Selection is incremental. Only the expected nonce for each account is eligible, so selecting nonce `n` may make nonce `n + 1` eligible during the same processing request. A transaction arriving after part of the proposal is already pre-confirmed can outrank transactions that have not yet been selected, but cannot reorder the published pre-confirmed prefix.

Queued `RECEIVED` and `CANDIDATE` transactions can be returned by `starknet_getTransactionByHash` and their phase can be returned by `starknet_getTransactionStatus`. They have no receipt or trace until they execute, and they are excluded from block and state-update queries.

## Inspect the pool

`devnet_getMempool` returns configuration, transactions in arrival order, the pre-confirmed transaction hashes, and remaining block capacity. Set `include_transactions` to `true` to include full transaction objects.

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "devnet_getMempool",
  "params": { "include_transactions": false }
}
```

## Select and pre-confirm transactions

Call `devnet_preconfirmTransactions` with no parameters to process policy-selected transactions until the proposal is full or no eligible transaction remains.

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "devnet_preconfirmTransactions"
}
```

Limit policy selection with `max_transactions`:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "devnet_preconfirmTransactions",
  "params": { "max_transactions": 25 }
}
```

For deterministic tests, supply an ordered list of hashes instead. Forced selection still enforces normal nonce eligibility, validation, and execution rules.

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "devnet_preconfirmTransactions",
  "params": { "transaction_hashes": ["0x1", "0x2"] }
}
```

The response separates `pre_confirmed`, `rejected`, and `blocked` hashes. Blocked transactions remain received; rejected transactions are removed; reverted executions are accepted and pre-confirmed with reverted receipts.

## Remove received transactions

Remove one received transaction with `devnet_removeFromMempool`:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "devnet_removeFromMempool",
  "params": { "transaction_hash": "0x1" }
}
```

Remove every received transaction with `devnet_clearMempool`. Neither method removes candidate or pre-confirmed transactions.

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "devnet_clearMempool"
}
```

## Change the policy at runtime

`devnet_setMempoolConfig` accepts a partial update. Ordering and seed changes affect the next selection. Capacity changes apply immediately and never discard transactions if the current proposal already exceeds the new capacity.

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "devnet_setMempoolConfig",
  "params": {
    "ordering": "random",
    "random_seed": 42,
    "max_transactions_per_block": 100
  }
}
```

## Seal or abort the proposal

`devnet_sealBlock` seals exactly the pre-confirmed prefix and leaves received transactions in the pool. `devnet_createBlock` first drains eligible transactions according to policy and then seals. Both permit an empty block.

`devnet_abortPreconfirmedBlock` reverts the speculative state and execution artifacts of the current proposal and moves its candidate and pre-confirmed transactions back to `RECEIVED`, preserving their arrival order. This is different from `devnet_abortBlocks`, which aborts accepted blocks.

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "devnet_abortPreconfirmedBlock"
}
```

## Restarting, dumping, and loading

Restarting clears the entire mempool and open proposal. Dumping records admission and each exact selected-hash sequence, configuration update, removal, clear, seal, and abort action so loading reproduces deterministic ordering without relying on timing or current policy defaults. Use the same startup account, class, block-generation, and mempool configuration when loading events into another Devnet instance.

## Future streaming builder

`mempool:<N>` is reserved for a future production-like streaming builder in which `N` is the maximum proposal-building duration rather than a periodic sealing cadence. This release rejects that value. The future scheduler will repeatedly call the same selection, preconfirmation, and strict-sealing operations documented here.
