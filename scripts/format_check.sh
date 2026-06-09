#!/bin/bash

set -euo pipefail

cargo +nightly-2026-04-10 fmt --all --check

# Format documentation
npm --prefix website run format-check
