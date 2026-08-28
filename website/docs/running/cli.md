---
sidebar_position: 2.3
---

# CLI options

Configure your Devnet instance by specifying CLI parameters on startup. To read more about HTTP and logging configuration, check out the [server config](../server-config) page.

## Help

Check out all the options with:

```
$ starknet-devnet --help
```

Or if using dockerized Devnet:

```
$ docker run --rm starknetfoundation/starknet-devnet-rs --help
```

## Environment variables

Every CLI option can also be specified via an environment variable:

```
$ <VAR1>=<VALUE> <VAR2>=<VALUE> starknet-devnet
```

To see the exact variable names, use [`--help`](#help).

## Block building and mempool configuration

`--block-generation-on transaction` executes and seals every submitted transaction immediately and remains the default. `demand` executes transactions into a live pre-confirmed block and seals only on request. `mempool` admits transactions as `RECEIVED` and waits for explicit [mempool and block-building requests](../mempool). A bare positive integer retains the deprecated periodic-sealing behavior for backward compatibility.

Configure manual mempool ordering and block capacity with:

```bash
$ starknet-devnet --block-generation-on mempool --mempool-ordering starknet --mempool-max-transactions-per-block 500
```

The available policies are `fifo`, `starknet`, and `random`. Use `--mempool-random-seed <SEED>` for reproducible random selection; it defaults to the Devnet account seed. Equivalent environment variables are `MEMPOOL_ORDERING`, `MEMPOOL_RANDOM_SEED`, and `MEMPOOL_MAX_TRANSACTIONS_PER_BLOCK`.

A bare positive integer `N` retains the deprecated periodic-sealing behavior for backward compatibility: transactions are pre-confirmed immediately and only block sealing occurs every N seconds. Prefer `--block-generation-on mempool` for Starknet-like policy-driven construction.

### Precedence

If both a CLI argument and an environment variable are passed for a parameter, the CLI argument takes precedence. If none are provided, the default value is used. E.g. if running Devnet with the following command, seed value 42 will be used:

```
$ SEED=10 starknet-devnet --seed 42
```

### Docker

If using dockerized Devnet, specify the variables like this:

```
$ docker run \
    -e <VAR1>=<VALUE> \
    -e <VAR2>=<VALUE> \
    ... \
    starknetfoundation/starknet-devnet-rs
```

## Load configuration from a file

If providing many configuration parameters in a single command becomes cumbersome, consider loading them from a file. By relying on [environment variables](#environment-variables), prepare your configuration in a file like this:

```bash
export SEED=42
export ACCOUNTS=3
...
```

Assuming the file is called `.my-env-file`, then run:

```bash
$ source .my-env-file && starknet-devnet
```

To run in a subshell and prevent environment pollution (i.e. to unset the variables after Devnet exits), use parentheses:

```bash
$ ( source .my-env-file && starknet-devnet )
```

### Docker

To load environment variables from `.my-env-file` with Docker, remove the `export` part in each line to have the file look like this:

```
SEED=42
ACCOUNTS=3
...
```

Then run:

```
$ docker run --env-file .my-env-file starknetfoundation/starknet-devnet-rs
```

## Proof-related configuration

Devnet exposes a dedicated proof-mode switch:

```bash
starknet-devnet --proof-mode <full|devnet|none>
```

Equivalent environment variable:

```bash
PROOF_MODE=<full|devnet|none>
```

Mode behavior summary:

- `devnet` (default): mock proof generation + verification flow is enabled.
- `none`: proof fields are ignored on invoke transactions.
- `full`: reserved for fully verified proofs (currently not implemented).

For complete examples and RPC payloads, see [Transaction proofs and proof modes](../proofs).
