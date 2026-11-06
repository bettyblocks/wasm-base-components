#!/usr/bin/env bash
#
# Select the components and WIT packages that CI must build: all of them, or with a base ref only
# the ones that changed since that base. A change to a shared build file selects all.
#
# Usage: ./scripts/ci-targets.sh [<base-ref>]
#
# Prints the lines `components=<json>` and `wit_packages=<json>` for $GITHUB_OUTPUT.

set -euo pipefail

# Files that every build reads.
SHARED='^(rust-toolchain\.toml|Justfile|wit-deps\.just|wkg-config\.toml|scripts/generate-wkg-toml\.sh|scripts/ci-targets\.sh|\.github/workflows/ci\.yml|\.github/actions/)'

main() {
  local base=${1:-} changed components wit_packages

  components=$(list_components)
  wit_packages=$(list_wit_packages)

  if [ -z "$base" ]; then
    echo "No base ref: select all." >&2
  else
    changed=$(git diff --name-only "$base" HEAD)
    if grep -qE "$SHARED" <<<"$changed"; then
      echo "A shared file changed: select all." >&2
    else
      components=$(keep_changed "$changed" <<<"$components")
      wit_packages=$(keep_changed "$changed" <<<"$wit_packages")
    fi
  fi

  echo "Components:   $(echo ${components:-(none)})" >&2
  echo "WIT packages: $(echo ${wit_packages:-(none)})" >&2

  echo "components=$(components_json <<<"$components")"
  echo "wit_packages=$(json_list <<<"$wit_packages")"
}

list_components() {
  find components -mindepth 2 -name Justfile -not -path '*/target/*' -printf '%h\n' | sort
}

list_wit_packages() {
  find wit -mindepth 1 -maxdepth 1 -type d | sort
}

# keep_changed <changed files>: keep the directories (stdin) that hold a changed file. The "/"
# makes sure that components/foo does not also keep components/foo-bar.
keep_changed() {
  local changed=$1 dir
  while read -r dir; do
    [ -n "$dir" ] || continue
    if grep -q "^$dir/" <<<"$changed"; then echo "$dir"; fi
  done
}

json_list() {
  jq -Rsc 'split("\n") | map(select(. != ""))'
}

components_json() {
  json_list | jq -c 'map({
    dir: .,
    name: sub("^components/"; ""),
    key: (sub("^components/"; "") | gsub("/"; "-"))
  })'
}

main "$@"
