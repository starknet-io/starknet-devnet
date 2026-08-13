#!/bin/bash

set -euo pipefail

nightly_toolchain="${NIGHTLY_TOOLCHAIN:-nightly-2026-04-10}"

command -v cargo >/dev/null || { echo "cargo is required" >&2; exit 1; }
cargo --version
cargo +"$nightly_toolchain" fmt --version

command -v node >/dev/null || { echo "node is required" >&2; exit 1; }
node --version

command -v npm >/dev/null || { echo "npm is required" >&2; exit 1; }
npm --version

if command -v anvil >/dev/null; then
    anvil --version
else
    echo "warning: anvil is required only for ./scripts/test_integration.sh"
fi

if command -v llvm-config-19 >/dev/null; then
    llvm-config-19 --version
else
    echo "warning: LLVM 19 is required for ./scripts/verify.sh and --all-features checks"
fi
