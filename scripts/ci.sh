#!/bin/bash

set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

"$script_dir/verify.sh"
"$script_dir/check_spelling.sh"
"$script_dir/test_integration.sh"
