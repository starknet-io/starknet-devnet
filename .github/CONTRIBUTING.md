# Contributing

Read the [Pull requests](#pull-requests) section for review expectations and [Development](#development) for setup and validation.

## Pull requests

Contributors own the quality of every submitted change. Keep changes focused, include appropriate tests and documentation, and review generated or assisted changes before submitting.

### Should you create a PR?

It is advised to [create an issue](https://github.com/starknet-io/starknet-devnet/issues/new/choose) before creating a PR. Creating an issue is the best way to reach somebody with repository-specific experience who can provide more info on how a problem/idea can be addressed and if a PR is needed.

### Checklist

The [PR template](pull_request_template.md) contains a checklist. It is important to go through the checklist to ensure the expected quality standards and to ensure the CI workflow succeeds once it is executed.

### Review

Once a PR is created, somebody from the team will review it. When a reviewer leaves a comment, the PR author should not mark the conversation as resolved. This is because the repository has a setting that prevents merging if there are unresolved conversations - let the reviewer resolve. The author can reply back with:

- a request for clarification from the reviewer
- a link to the commit which addresses the reviewer's observation (simply pasting the sha-digest is enough)

This is an example of a good author-reviewer correspondence: [link](https://github.com/starknet-io/starknet-devnet/pull/310#discussion_r1457142002).

## Development

The root [AGENTS.md](../AGENTS.md) is the task-oriented guide for coding agents and other automated contributors. It documents the repository layout, generated files, and expectations for a handoff.

### Standard commands

The repository's `scripts/` directory contains the supported development commands.

| Command | Purpose |
| --- | --- |
| `./scripts/doctor.sh` | Check the local tools required for common workflows. |
| `./scripts/format.sh` / `./scripts/format_check.sh` | Apply or check Rust and website formatting. |
| `cargo check --workspace --all-targets --all-features` / `./scripts/clippy_check.sh` | Type-check all Rust targets and run the required Clippy checks. |
| `cargo test --workspace --exclude integration --no-fail-fast` | Run unit tests; integration tests are excluded. |
| `./scripts/test_integration.sh` | Build the release binary and run integration tests. |
| `./scripts/check_web_ui.sh` | Lint and rebuild the embedded UI, then verify its committed assets. |
| `./scripts/check_website.sh` | Type-check and build the documentation site after `npm --prefix website ci`. |
| `./scripts/verify.sh` | Run routine checks that do not require Foundry. |
| `./scripts/ci.sh` | Run the full local CI-equivalent suite. |

### Prerequisites

- Rust stable is pinned in `rust-toolchain.toml`. Formatting and spelling additionally require the pinned nightly toolchain:

  ```
  $ rustup toolchain install nightly-2026-04-10 --component rustfmt
  ```

- Use Node 24 and npm 11 to match CI.
- Integration tests require [Foundry](https://book.getfoundry.sh/getting-started/installation); `anvil` must be on `PATH`.
- All-features Rust checks require LLVM 19. In CI, `MLIR_SYS_190_PREFIX`, `LLVM_SYS_191_PREFIX`, and `TABLEGEN_190_PREFIX` point to `/usr/lib/llvm-19`.

### Installation

Run `./scripts/doctor.sh` after installing the prerequisites. It checks the required Rust and Node tooling and reports optional integration-test dependencies. The website and web UI commands use `npm ci`, so their lockfiles are respected.

### Editor support

Any editor with Rust Analyzer support works well. Ensure editor-launched tests inherit the shell `PATH` that contains `anvil` when testing integration code.

### Linter

Run the linter with:

```
$ ./scripts/clippy_check.sh
```

### Formatter

Run the formatter with:

```
$ npm --prefix website ci && ./scripts/format.sh
```

If you encounter an error like

```
error: toolchain 'nightly-2026-04-10' is not installed
```

Resolve it with:

```
$ rustup toolchain install nightly-2026-04-10 --component rustfmt
```

### Unused dependencies

To check for unused dependencies, run:

```
$ ./scripts/check_unused_deps.sh
```

If you think this reports a dependency as a false positive (i.e. isn't unused), check [here](https://github.com/bnjbvr/cargo-machete#false-positives).

### Spelling check

To check for spelling errors in the code, run:

```
$ ./scripts/check_spelling.sh
```

If you think this reports a false-positive, check [here](https://crates.io/crates/typos-cli#false-positives).

### Pre-commit

To speed up development, you can put the previous steps (and more) in a local script defined at `.git/hooks/pre-commit` to have it run before each commit ([more info](https://git-scm.com/book/en/v2/Customizing-Git-Git-Hooks)).

### Testing

#### Prerequisites

Integration tests require the `anvil` command from [Foundry](https://book.getfoundry.sh/getting-started/installation). Run them from a shell whose `PATH` contains `anvil`.

#### Test execution

Run the unit-test suite with:

```
$ cargo test --workspace --exclude integration --no-fail-fast
```

Run the integration suite after production, RPC, CLI, or contract-fixture changes with:

```
$ ./scripts/test_integration.sh
```

Integration tests build the release binary and can take longer after production-code changes. If resources are constrained, pass `--jobs=<N>` to a focused Cargo command.

#### Benchmarking

To test if your contribution presents an improvement in execution time, check out the script at `scripts/benchmark/command_stat_test.py`.

## Updating versions

Generally, when updating to a new version of something (a spec file, a contract artifact, ...), a good rule of thumb is to search the repository for mentions of the old version, both in file names and content. This should also aid in not forgetting to update version mentions in the documentation.

### Updating OpenZeppelin contracts

Devnet requires an ERC20 contract with the `Mintable` feature; keep in mind that before the local compilation of [cairo-contracts](https://github.com/OpenZeppelin/cairo-contracts/) you need to mark the `Mintable` check box in this [wizard](https://wizard.openzeppelin.com/cairo) and copy the generated file to `packages/presets/src/erc20.cairo` of your local Open Zeppelin repository.

If smart contract constructor logic has changed, Devnet's predeployment logic needs to be changed, e.g. `simulate_constructor` in `crates/starknet-devnet-core/src/account.rs`.

### Updating Starknet

Updating the underlying Starknet is done by updating the `blockifier` and `starknet_api` dependencies from the [`sequencer` repo](https://github.com/starkware-libs/sequencer/) and addressing changes. Other dependencies might also need to be updated. Sometimes, `blockifier` may not yet be ready, so its development branch or git tag might need to be used. This is acceptable during development, but will prevent Devnet from being releasable on crates.io, as all dependencies for that must also be on crates.io. Devnet maintainers may choose to make pre-releases not available on crates.io.

Starknet adaptation also requires updating the `STARKNET_VERSION` constant and the used `versioned_constants`.

### Updating JSON-RPC API

Updating the RPC requires following the specification files in the [starknet-specs repository](https://github.com/starkware-libs/starknet-specs). The spec_reader testing utility requires these files to be copied into the Devnet repository. The `RPC_SPEC_VERSION` constant needs to be updated accordingly.

Integration tests highly depend on [starknet-rs](https://github.com/xJonathanLEI/starknet-rs) supporting the same JSON-RPC API version as Devnet. Until an adapted starknet-rs version is released, Devnet maintainers can rely on replacing the starknet-rs dependencies in tests/integration/Cargo.toml with links to SpaceShard's fork of starknet-rs. A full Devnet can be released on crates.io even with such git dependencies because the integration crate is not released. An example of such an adapted branch on SpaceShard's fork is [this](https://github.com/starknet-io/starknet-rs/tree/rpc-0.9).

### Adding new dependencies

When adding new Rust dependencies, specify them in the root Cargo.toml and use `{ workspace = true }` in crate-specific Cargo.toml files.

### Adding new artifacts

When adding new compilation artifacts, e.g. in the format of JSON files, please minify them to reduce: your footprint, the codebase size, artifact loading time. This can be achieved using your IDE's minifier tool/plugin. This doesn't apply to JSON-RPC spec files.

### Updating documentation

The documentation website content has [its own readme](../website/README.md).

### Releasing

To release a new version, check out the [release docs](../RELEASE.md).
