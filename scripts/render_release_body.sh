#!/usr/bin/env bash
# Renders the Markdown body used for published GitHub releases.
#
# Required environment variables:
#   VERSION          - e.g. "0.8.0-rc.3" or "0.8.0"
#   SHA              - short commit SHA used for the Docker image tag
#   IS_RC            - "true" if the version is a release candidate, else "false"
#   DOCKER_REGISTRY  - e.g. "docker.io"
#   DOCKER_NAMESPACE - e.g. "shardlabs"
#   IMAGE_NAME       - e.g. "starknet-devnet-rs"
#   OUTPUT_PATH      - path where the rendered body is written
#
# The script intentionally avoids every GitHub Actions `${{ ... && '...' }}`
# ternary that produced the v0.8.0-rc.* malformed Docker notes (see issue #969)
# by evaluating the RC branching as a plain shell `if/else`.

set -euo pipefail

: "${VERSION:?VERSION is required}"
: "${SHA:?SHA is required}"
: "${IS_RC:?IS_RC is required}"
: "${DOCKER_REGISTRY:?DOCKER_REGISTRY is required}"
: "${DOCKER_NAMESPACE:?DOCKER_NAMESPACE is required}"
: "${IMAGE_NAME:?IMAGE_NAME is required}"
: "${OUTPUT_PATH:?OUTPUT_PATH is required}"

mkdir -p "$(dirname "$OUTPUT_PATH")"

{
  printf '# Starknet Devnet v%s\n\n' "$VERSION"
  printf '## Installation\n\n'
  printf '### Binary\n'
  printf 'Download the appropriate binary for your platform from the assets below.\n\n'
  printf '### Docker\n'
  printf '```bash\n'
  printf '# Pull by version\n'
  printf 'docker pull %s/%s/%s:%s\n' "$DOCKER_REGISTRY" "$DOCKER_NAMESPACE" "$IMAGE_NAME" "$VERSION"
  printf 'docker pull %s/%s/%s:%s-seed0\n\n' "$DOCKER_REGISTRY" "$DOCKER_NAMESPACE" "$IMAGE_NAME" "$VERSION"
  printf '# Pull by SHA\n'
  printf 'docker pull %s/%s/%s:sha-%s\n' "$DOCKER_REGISTRY" "$DOCKER_NAMESPACE" "$IMAGE_NAME" "$SHA"
  printf 'docker pull %s/%s/%s:sha-%s-seed0\n' "$DOCKER_REGISTRY" "$DOCKER_NAMESPACE" "$IMAGE_NAME" "$SHA"
  printf '```\n'

  if [[ "$IS_RC" != "true" ]]; then
    printf '\n# Pull latest versions\n'
    printf 'docker pull %s/%s/%s:latest\n' "$DOCKER_REGISTRY" "$DOCKER_NAMESPACE" "$IMAGE_NAME"
    printf 'docker pull %s/%s/%s:latest-seed0\n' "$DOCKER_REGISTRY" "$DOCKER_NAMESPACE" "$IMAGE_NAME"
  else
    printf '\n### Note\n\nLatest tags are not available for release candidates.\n'
  fi

  printf '\n### Cargo\n'
  printf '```bash\n'
  printf 'cargo install starknet-devnet\n'
  printf '```\n'
} > "$OUTPUT_PATH"
