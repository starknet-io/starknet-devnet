#!/bin/bash

set -eu

cargo +nightly-2026-04-10 fmt --all

# Format documentation
npm --prefix website run format
