#!/bin/bash

set -euo pipefail

npm --prefix web-ui ci
npm --prefix web-ui run lint
npm --prefix web-ui run build:devnet
git diff --exit-code -- crates/starknet-devnet-server/assets/ui
test -z "$(git status --porcelain -- crates/starknet-devnet-server/assets/ui)"
