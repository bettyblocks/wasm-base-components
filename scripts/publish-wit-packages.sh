#!/usr/bin/env bash
#
# Build and push the lean WIT packages under wit/<name>/ to one registry.
#
# These come from the checkout, not from the build bundle, so there is nothing to check them
# against — the WIT *is* the source. The version is each package's own
# `package betty-blocks-types:<name>@X.Y.Z;` declaration, never a git tag.
#
# Env:
#   REGISTRY  required  host and namespace, e.g. myreg.azurecr.io/betty-blocks-types
#   DRY_RUN   optional  true to print instead of pushing

set -uo pipefail

: "${REGISTRY:?REGISTRY is required (host and namespace, e.g. myreg.azurecr.io/betty-blocks-types)}"
DRY_RUN=${DRY_RUN:-false}

main() {
  build_dir=$(mktemp -d)
  result_code=0
  found_package=0
  name=""
  version=""
  shopt -s nullglob

  for package_dir in wit/*/; do
    package_dir=${package_dir%/}

    read_package_declaration "$package_dir" name version
    [ -n "$name" ] || continue # not a betty-blocks-types package; nothing to publish
    found_package=1

    if [ -z "$version" ]; then
      echo "::error::$package_dir: package betty-blocks-types:$name has no @version"
      result_code=1
      continue
    fi

    if ! build_wit_package "$package_dir" "$name"; then
      echo "::error::$name: wkg wit build failed in $package_dir"
      result_code=1
      continue
    fi

    push_wit_package "$name" "$version" "$build_dir/$name.wasm" "$package_dir" || result_code=1
  done

  if [ "$found_package" -eq 0 ]; then
    echo "::error::no betty-blocks-types packages found under wit/ — is this the right checkout?"
    exit 1
  fi

  exit $result_code
}

read_package_declaration() {
  local package_dir=$1
  local -n _name=$2 _version=$3
  local wit
  _name=""
  _version=""

  for wit in "$package_dir"/*.wit; do
    _name=$(sed -nE 's/^package betty-blocks-types:([a-z0-9-]+).*/\1/p' "$wit" | head -1)
    [ -n "$_name" ] || continue
    _version=$(sed -nE 's/^package[^@]*@([^;[:space:]]+).*/\1/p' "$wit" | head -1)
    break
  done
}

build_wit_package() {
  local package_dir=$1 name=$2
  (cd "$package_dir" && wkg wit build --wit-dir . -o "$build_dir/$name.wasm")
}

append_to_summary() {
  [ -n "${GITHUB_STEP_SUMMARY:-}" ] || return 0
  echo "$1" >>"$GITHUB_STEP_SUMMARY"
}

push_wit_package() {
  local name=$1 version=$2 wasm=$3 package_dir=$4
  local image="$REGISTRY/$name"

  if [ "$DRY_RUN" = true ]; then
    echo "would push $image:$version  <-  $package_dir"
    return 0
  fi

  if wkg oci push "$image:$version" "$wasm"; then
    echo "Published $image:$version"
    append_to_summary "- \`$image:$version\`"
    return 0
  fi

  echo "::error::$name: failed to push $image:$version"
  append_to_summary "- ~\`$image:$version\`~ **push failed**"
  return 1
}

main "$@"
