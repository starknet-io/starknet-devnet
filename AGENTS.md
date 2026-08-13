# Agent Guide

## Purpose and scope

This guide applies to the whole repository. Starknet Devnet is a Rust workspace with an embedded React UI and a Docusaurus documentation site. Prefer the repository's commands over ad-hoc equivalents so local validation matches CI.

## Markdown line wrapping

Never hard-wrap Markdown prose. A paragraph must remain on a single source line, regardless of its length. Do not insert newlines at 80, 100, 120, or any other column width.

Newlines are allowed only when they are semantically required by Markdown structure, such as headings, lists, blockquotes, code blocks, tables, or paragraph separation.

## Fast path

```sh
./scripts/doctor.sh
./scripts/verify.sh
```

Run `./scripts/test_integration.sh` when a change affects the running server, RPC behavior, contracts, or integration tests. It requires Foundry's `anvil`. `./scripts/verify.sh` (and therefore `./scripts/ci.sh`) requires the native dependencies needed by its all-features Rust checks; `./scripts/doctor.sh` only warns when optional tools are missing and still exits 0. `./scripts/ci.sh` also runs spelling and integration tests.

Use the focused scripts in `scripts/` for individual checks. State which targets you ran in the final handoff and name any skipped target with its reason.

## Toolchain

- The Rust toolchain is declared in `rust-toolchain.toml`.
- The Rust nightly, Node.js, and npm versions used by CI are defined in `.github/actions/load-config/action.yml`.
- Formatting and spelling scripts use the configured nightly by default; set `NIGHTLY_TOOLCHAIN` only when an explicit override is required.
- Integration tests need Foundry's `anvil` on `PATH`; the CI version is configured in `.github/workflows/build-and-test.yml`.
- The all-features Rust checks require the LLVM configuration exported by the CI workflow. Set equivalent paths on other platforms when needed.

Do not edit `Cargo.lock` or generated web UI assets by hand.

## Repository map

- `crates/starknet-devnet`: CLI binary and command-line configuration.
- `crates/starknet-devnet-server`: HTTP/RPC server and embedded UI asset packaging.
- `crates/starknet-devnet-core`: Devnet state, transactions, and execution logic.
- `crates/starknet-devnet-types`: shared RPC and Starknet types.
- `tests/integration`: black-box tests that launch the release binary.
- `web-ui`: React/Vite UI source; its compiled output is committed in `crates/starknet-devnet-server/assets/ui`.
- `website`: Docusaurus documentation. Edit current docs in `website/docs`; only amend `website/versioned_docs` when a correction applies to a released version.
- `contracts`: Cairo and Solidity fixtures used by tests.

## Change workflow

1. Inspect the nearest production code and existing focused tests before changing behavior. Keep the change narrow and preserve public RPC/CLI behavior unless the task says otherwise.
2. Add a focused unit test for isolated Rust logic. Add or update an integration test for externally observable JSON-RPC, CLI, server, or process behavior.
3. Put new Rust dependencies in the root `Cargo.toml` workspace dependencies and reference them as `{ workspace = true }` from member crates.
4. Run the smallest relevant check first, then `./scripts/verify.sh`; run `./scripts/test_integration.sh` when required by the change. Use `./scripts/format.sh` only to apply formatting and `./scripts/format_check.sh` in validation.
5. Update the current website docs and/or CLI help when behavior visible to users changes.

## Generated and sensitive areas

- After changing `web-ui`, run `npm --prefix web-ui ci` and `npm --prefix web-ui run build:devnet`, then include the resulting changes under `crates/starknet-devnet-server/assets/ui`. `./scripts/check_web_ui.sh` verifies that those generated assets match the committed files.
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
