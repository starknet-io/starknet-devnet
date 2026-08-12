#!/bin/bash

set -euo pipefail

nightly_toolchain="${NIGHTLY_TOOLCHAIN:-nightly-2026-04-10}"

# should skip if already installed
cargo +"$nightly_toolchain" install typos-cli --version 1.43.4 --locked

typos && echo "No spelling errors!"
