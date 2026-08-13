#!/bin/bash

set -euo pipefail

cargo build --release
RUST_BACKTRACE=full cargo test --package integration --no-fail-fast
