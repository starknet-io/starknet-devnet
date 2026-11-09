# Historic state support

With state archive capacity set to `full`, Devnet will store full state history, enabling its querying by block hash or number. The default mode is `none`, where no old states are stored; the latest accepted state and the pre-confirmed state remain available for querying. In-memory snapshots work with either archive mode.

```
$ starknet-devnet --state-archive-capacity <CAPACITY>
```

Querying contract state at an older accepted block requires state archive capacity `full`. Retained blocks, transactions, receipts, and traces can still be queried without full state history.
