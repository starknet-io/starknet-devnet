#!/bin/bash

set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"
cd "$repo_root"

npm --prefix website ci
"$script_dir/format_check.sh"
"$script_dir/check_web_ui.sh"
"$script_dir/check_website.sh"
cargo check --workspace --all-targets --all-features
"$script_dir/clippy_check.sh"
RUST_BACKTRACE=full cargo test --workspace --exclude integration --no-fail-fast
