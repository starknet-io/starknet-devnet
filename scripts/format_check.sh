#!/bin/bash

set -euo pipefail

nightly_toolchain="${NIGHTLY_TOOLCHAIN:-nightly-2026-04-10}"

cargo +"$nightly_toolchain" fmt --all --check

# Format documentation
npm --prefix website run format-check
