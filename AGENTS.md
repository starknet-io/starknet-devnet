# Agent Guide

## Purpose and scope

This guide applies to the whole repository. Starknet Devnet is a Rust workspace with an
embedded React UI and a Docusaurus documentation site. Prefer the repository's commands
over ad-hoc equivalents so local validation matches CI.

## Fast path

```sh
make doctor
make verify
```

Run `make integration` when a change affects the running server, RPC behavior, contracts,
or integration tests. It requires Foundry's `anvil`. `make verify` (and therefore `make ci`)
requires LLVM 19 for the `--all-features` Rust checks in `check`/`lint`; `make doctor` only
warns when LLVM is missing and still exits 0. `make ci` also runs spelling and integration.

Use `make help` to see every target. State which targets you ran in the final handoff and
name any skipped target with its reason.

## Toolchain

- Rust stable is pinned in `rust-toolchain.toml` (currently 1.96.0).
- Formatting and spelling use `nightly-2026-04-10`; install it with
  `rustup toolchain install nightly-2026-04-10 --component rustfmt`.
- Use Node 24 and npm 11 to match CI. The package manifests support Node 18 or newer, but
  the CI versions are the preferred baseline.
- Integration tests need Foundry's `anvil` on `PATH`.
- `cargo check --all-features` and related commands require LLVM 19. CI exports
  `MLIR_SYS_190_PREFIX`, `LLVM_SYS_191_PREFIX`, and `TABLEGEN_190_PREFIX` as
  `/usr/lib/llvm-19`; set equivalent paths on other platforms when needed.

Do not edit `Cargo.lock` or generated web UI assets by hand.

## Repository map

- `crates/starknet-devnet`: CLI binary and command-line configuration.
- `crates/starknet-devnet-server`: HTTP/RPC server and embedded UI asset packaging.
- `crates/starknet-devnet-core`: Devnet state, transactions, and execution logic.
- `crates/starknet-devnet-types`: shared RPC and Starknet types.
- `tests/integration`: black-box tests that launch the release binary.
- `web-ui`: React/Vite UI source; its compiled output is committed in
  `crates/starknet-devnet-server/assets/ui`.
- `website`: Docusaurus documentation. Edit current docs in `website/docs`; only amend
  `website/versioned_docs` when a correction applies to a released version.
- `contracts`: Cairo and Solidity fixtures used by tests.

## Change workflow

1. Inspect the nearest production code and existing focused tests before changing behavior.
   Keep the change narrow and preserve public RPC/CLI behavior unless the task says otherwise.
2. Add a focused unit test for isolated Rust logic. Add or update an integration test for
   externally observable JSON-RPC, CLI, server, or process behavior.
3. Put new Rust dependencies in the root `Cargo.toml` workspace dependencies and reference
   them as `{ workspace = true }` from member crates.
4. Run the smallest relevant target first, then `make verify`; run `make integration` when
   required by the change. Use `make format` only to apply formatting and `make format-check`
   in validation.
5. Update the current website docs and/or CLI help when behavior visible to users changes.

## Generated and sensitive areas

- After changing `web-ui`, run `npm --prefix web-ui ci` and
  `npm --prefix web-ui run build:devnet`, then include the resulting changes under
  `crates/starknet-devnet-server/assets/ui`. `make web-ui` is a verification target: it
  intentionally fails if those generated assets differ from the committed files.
- JSON compilation artifacts should be minified unless they are JSON-RPC specification files.
- Never run release or publishing scripts as part of ordinary development.
- Avoid broad formatter or dependency upgrades unless the task specifically calls for them.

## Useful focused commands

```sh
# One crate or test module
cargo test -p starknet-devnet-core <test-name>
cargo test -p integration <test-name> -- --nocapture

# Run the devnet binary locally
cargo run --bin starknet-devnet -- --help

# Website development
npm --prefix website ci
npm --prefix website run start

# Web UI development; build generated assets before committing UI changes
npm --prefix web-ui ci
npm --prefix web-ui run dev
```
