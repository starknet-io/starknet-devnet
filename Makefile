.DEFAULT_GOAL := help

CARGO ?= cargo
NPM ?= npm
NIGHTLY_TOOLCHAIN ?= nightly-2026-04-10

.PHONY: \
	help doctor format format-check check lint test integration \
	web-ui website unused-deps spelling verify ci

help: ## List the available development commands.
	@awk 'BEGIN {FS = ":.*##"}; /^[a-zA-Z0-9_-]+:.*##/ { printf "%-16s %s\n", $$1, $$2 }' $(MAKEFILE_LIST)

doctor: ## Check the local tools required for the common workflows.
	@command -v $(CARGO) >/dev/null || { echo "cargo is required" >&2; exit 1; }
	@$(CARGO) --version
	@$(CARGO) +$(NIGHTLY_TOOLCHAIN) fmt --version
	@command -v node >/dev/null || { echo "node is required" >&2; exit 1; }
	@node --version
	@command -v $(NPM) >/dev/null || { echo "npm is required" >&2; exit 1; }
	@$(NPM) --version
	@if command -v anvil >/dev/null; then anvil --version; else echo "warning: anvil is required only for make integration"; fi
	@if command -v llvm-config-19 >/dev/null; then llvm-config-19 --version; else echo "warning: LLVM 19 is required for --all-features checks"; fi

format: ## Format Rust and website sources.
	$(NPM) --prefix website ci
	NIGHTLY_TOOLCHAIN=$(NIGHTLY_TOOLCHAIN) ./scripts/format.sh

format-check: ## Check Rust and website formatting without changing files.
	$(NPM) --prefix website ci
	NIGHTLY_TOOLCHAIN=$(NIGHTLY_TOOLCHAIN) ./scripts/format_check.sh

check: ## Type-check every Rust target and feature.
	$(CARGO) check --workspace --all-targets --all-features

lint: ## Run the Rust linter with the repository's required flags.
	./scripts/clippy_check.sh

test: ## Run unit tests (excludes integration tests that require anvil).
	RUST_BACKTRACE=full $(CARGO) test --workspace --exclude integration --no-fail-fast

integration: ## Build the release binary and run integration tests (requires anvil).
	$(CARGO) build --release
	RUST_BACKTRACE=full $(CARGO) test --package integration --no-fail-fast

web-ui: ## Lint and rebuild the embedded web UI, then verify its committed assets.
	$(NPM) --prefix web-ui ci
	$(NPM) --prefix web-ui run lint
	$(NPM) --prefix web-ui run build:devnet
	git diff --exit-code -- crates/starknet-devnet-server/assets/ui
	test -z "$$(git status --porcelain -- crates/starknet-devnet-server/assets/ui)"

website: ## Type-check and build the documentation website.
	$(NPM) --prefix website ci
	$(NPM) --prefix website run typecheck
	$(NPM) --prefix website run build

unused-deps: ## Check Rust manifests for unused dependencies.
	./scripts/check_unused_deps.sh

spelling: ## Check spelling (installs the pinned typos CLI if necessary).
	NIGHTLY_TOOLCHAIN=$(NIGHTLY_TOOLCHAIN) ./scripts/check_spelling.sh

verify: format-check web-ui website check lint test ## Run the checks that do not require Foundry.

ci: verify spelling integration ## Run the full CI-equivalent validation suite.
