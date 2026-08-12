#!/bin/bash

set -eu

nightly_toolchain="${NIGHTLY_TOOLCHAIN:-nightly-2026-04-10}"

cargo +"$nightly_toolchain" fmt --all

# Format documentation
npm --prefix website run format
