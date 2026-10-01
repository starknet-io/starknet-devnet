# Restrictive mode

The `--restrictive-mode` argument enables a restrictive mode for Devnet, allowing you to specify methods that are forbidden during execution. This option ensures that certain operations are restricted, enhancing control over Devnet's behavior. When a user sends a request to one of the restricted methods, Devnet returns a JSON-RPC error with code `-32604` over both HTTP and WebSocket.

## Default restricted methods

When no methods are specified, the following JSON-RPC methods are restricted:

- devnet_mint
- devnet_load
- devnet_snapshot
- devnet_revert
- devnet_restart
- devnet_createBlock
- devnet_abortBlocks
- devnet_impersonateAccount
- devnet_autoImpersonate
- devnet_getPredeployedAccounts

## Usage

### With default methods

```
$ starknet-devnet --restrictive-mode
```

### With a list of methods

:::note

Devnet will fail to start if any of the methods are misspelled.

:::

```
$ starknet-devnet --restrictive-mode devnet_dump devnet_getConfig
```
