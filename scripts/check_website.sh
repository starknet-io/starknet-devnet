#!/bin/bash

set -euo pipefail

npm --prefix website run typecheck
npm --prefix website run build
