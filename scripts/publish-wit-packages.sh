#!/usr/bin/env bash
#
# Build and push the lean WIT packages under wit/<name>/ to one registry.
#
# These come from the checkout, not from the build bundle, so there is nothing to check them
# against — the WIT *is* the source. The version is each package's own
# `package betty-blocks-types:<name>@X.Y.Z;` declaration, never a git tag.
#
# Env:
#   REGISTRY          required  host and namespace, e.g. myreg.azurecr.io/betty-blocks-types
#   REGISTRY_USERNAME optional  identity for *reading* dependencies out of REGISTRY's host.
#   REGISTRY_PASSWORD optional  It must be able to list tags; unset means read anonymously,
#                               and either way the push keeps the docker login (see
#                               setup_wkg_config).
#   DRY_RUN           optional  true to print instead of pushing
#   WKG_CONFIG_FILE   optional  registry config; overrides the one generated per run

set -uo pipefail

: "${REGISTRY:?REGISTRY is required (host and namespace, e.g. myreg.azurecr.io/betty-blocks-types)}"
DRY_RUN=${DRY_RUN:-false}
registry_host=${REGISTRY%%/*}
anonymous_docker_config=""

# A package whose dependency this checkout does not hold at the declared version resolves it
# from the registry, so building one needs a registry config, the generated wkg.toml, and a
# working read path into the registry.
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

main() {
  setup_wkg_config
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

# Dependencies resolve from the host this run publishes to, not from the committed
# wkg-config.toml, which names dev — a production publish must not read its deps out of dev.
#
# The read deliberately does not inherit the push login: listing tags is an ACR metadata/read
# that a content/read + content/write token lacks, so it 401s where anonymous succeeds. Set
# REGISTRY_USERNAME/REGISTRY_PASSWORD for an identity that can list tags; unset reads anonymously.
setup_wkg_config() {
  local auth

  if [ -n "${WKG_CONFIG_FILE:-}" ]; then
    echo "Resolving dependencies with $WKG_CONFIG_FILE"
    export WKG_CONFIG_FILE
    return 0
  fi

  wkg_config=$(mktemp)
  trap 'rm -f "$wkg_config"; [ -n "$anonymous_docker_config" ] && rm -rf "$anonymous_docker_config"' EXIT

  {
    # wasi is listed explicitly: naming a config file replaces wkg's built-in namespace
    # defaults rather than adding to them, so leaving it out breaks every wasi: import.
    echo '[namespace_registries]'
    echo 'wasi = "wasi.dev"'
    echo "betty-blocks-types = \"$registry_host\""

    # base64 "user:password", so a password holding quotes or backslashes needs no escaping.
    if [ -n "${REGISTRY_USERNAME:-}" ] && [ -n "${REGISTRY_PASSWORD:-}" ]; then
      auth=$(printf '%s:%s' "$REGISTRY_USERNAME" "$REGISTRY_PASSWORD" | base64 -w0)
      echo
      echo "[registry.\"$registry_host\".oci]"
      echo "auth = \"$auth\""
    fi
  } >"$wkg_config"

  export WKG_CONFIG_FILE=$wkg_config

  if [ -n "${REGISTRY_USERNAME:-}" ] && [ -n "${REGISTRY_PASSWORD:-}" ]; then
    echo "Resolving betty-blocks-types dependencies from $registry_host as $REGISTRY_USERNAME"
    return 0
  fi

  # With no credentials in the config wkg falls back to ~/.docker/config.json, which by this
  # point holds the push login. An empty DOCKER_CONFIG leaves it nothing to find, so it asks
  # anonymously instead of with an identity that cannot list tags.
  anonymous_docker_config=$(mktemp -d)
  echo '{}' >"$anonymous_docker_config/config.json"
  echo "Resolving betty-blocks-types dependencies from $registry_host anonymously"
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

# Only the build resolves dependencies, so only the build runs under the read identity; the
# push that follows keeps the runner's own docker login.
build_wit_package() {
  local package_dir=$1 name=$2
  (
    [ -n "$anonymous_docker_config" ] && export DOCKER_CONFIG="$anonymous_docker_config"
    cd "$package_dir" &&
      "$repo_root/scripts/generate-wkg-toml.sh" &&
      wkg wit build --wit-dir . -o "$build_dir/$name.wasm"
  )
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
