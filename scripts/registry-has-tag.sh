#!/usr/bin/env bash
#
# Tell whether a tag already exists in a registry.
#
# Usage: ./scripts/registry-has-tag.sh <registry>/<name>:<tag>
#
# Exit 0: the tag exists. Exit 1: the tag does not exist. Exit 2: the registry could not be read.
# Uses the docker credentials of the runner (docker/login-action writes them).

set -uo pipefail

ref=${1:?usage: registry-has-tag.sh <registry>/<name>:<tag>}

if output=$(docker manifest inspect "$ref" 2>&1 >/dev/null); then
  exit 0
fi

# A missing tag and a missing repository both mean the tag does not exist yet.
if grep -qiE 'manifest unknown|name unknown|no such manifest|not found' <<<"$output"; then
  exit 1
fi

echo "::error::cannot read $ref from the registry: $output" >&2
exit 2
