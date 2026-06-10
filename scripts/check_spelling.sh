#!/bin/bash

set -euo pipefail

# should skip if already installed
cargo +nightly-2026-04-10 install typos-cli --version 1.43.4 --locked

typos && echo "No spelling errors!"
