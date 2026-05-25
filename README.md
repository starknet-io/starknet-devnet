<!-- logo / title -->
<p align="center" style="margin-bottom: 0px !important">
  <img width="200" src="https://starknet-io.github.io/starknet-devnet/img/devnet-logo.png" alt="Devnet-RS" align="center">
</p>

<h1 align="center" style="margin-top: 12px !important">Starknet Devnet RS</h1>

<p align="center" dir="auto">
  <a href="https://crates.io/crates/starknet-devnet" target="_blank">
    <img src="https://img.shields.io/crates/v/starknet-devnet?color=yellow" style="max-width: 100%;">
  </a>
  <a href="https://hub.docker.com/r/shardlabs/starknet-devnet-rs/tags" target="_blank">
    <img src="https://img.shields.io/badge/dockerhub-images-important.svg?logo=Docker" style="max-width: 100%;">
  </a>
  <a href="https://starkware.co/" target="_blank">
    <img src="https://img.shields.io/badge/powered_by-StarkWare-navy" style="max-width: 100%;">
  </a>
</p>

A local testnet for Starknet... in Rust!

## Features

- [Forking](https://starknet-io.github.io/starknet-devnet/docs/forking) - interact with contracts deployed on mainnet or testnet
- [Account impersonation](https://starknet-io.github.io/starknet-devnet/docs/account-impersonation)
- [L1-L2 interaction](https://starknet-io.github.io/starknet-devnet/docs/postman)
- [Predeployed contracts](https://starknet-io.github.io/starknet-devnet/docs/predeployed) - accounts, tokens etc.
- [Block manipulations](https://starknet-io.github.io/starknet-devnet/docs/blocks) - creation, abortion etc.
- [Time manipulations](https://starknet-io.github.io/starknet-devnet/docs/starknet-time/)
- [Dump, load, restart state](https://starknet-io.github.io/starknet-devnet/docs/dump-load-restart)
- [Configurable according to your needs](https://starknet-io.github.io/starknet-devnet/docs/running/cli)

## 🌐 Documentation

Find the official documentation [here](https://starknet-io.github.io/starknet-devnet/).

## cairo_native execution (feature flag)

Devnet can run blockifier with native execution via the `cairo_native` Cargo feature. Enable it when building or running:

- Build: `cargo build --features cairo_native`
- Run: `cargo run --features cairo_native -- <args>`

When enabled, startup logs include `cairo_native enabled: blockifier will use native execution`.

## starknet-devnet-js

Simplify the installation, spawning and usage of Devnet in your tests by relying on the official JavaScript wrapper. Read more [here](https://github.com/starknet-io/starknet-devnet-js).

## ✏️ Contributing

We ❤️ and encourage all contributions and thank all the [contributors](https://github.com/starknet-io/starknet-devnet/graphs/contributors)!

[Click here](.github/CONTRIBUTING.md) for the development guide.
