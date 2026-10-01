# Cairo Contracts

This folder contains a Scarb package to compile and deploy Cairo 1 contracts on Devnet for development purposes.

## Work with Scarb

Install Scarb [from the tutorial](https://docs.swmansion.com/scarb/). The package declares Starknet `>=2.3.1` in `Scarb.toml`; use a compatible compiler.

### Build

To build contracts from this directory, use:

```bash
scarb build
```

The contract artifacts are generated into `target/dev`. Each contract produces:

- The Sierra class file: `cairo_<contract-name>.contract_class.json`
- The compiled CASM file: `cairo_<contract-name>.compiled_contract_class.json`

### Interact with Devnet

To interact with Devnet, you can use [Starkli](https://book.starkli.rs/). Configure an account file containing the deployed account definition and address, along with a keystore or a private key. The [parent guide](../README.md) uses the bundled test account and a Devnet started with `--seed 42 --account-class cairo0`.
