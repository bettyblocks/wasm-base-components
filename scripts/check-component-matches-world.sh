#!/usr/bin/env bash
#
# Check that a built component was really built from a given world.wit — the only step in the
# publish pipeline that looks at the actual bytes, so a @2.0.0 binary cannot ship as :3.0.0.
#
# A world and its component do not have identical interface lists, so only three rules hold:
#
#   1. Interfaces both sides name in the same direction must agree on version.
#   2. Every non-wasi export the world declares must be present in the .wasm.
#   3. The two sides must share at least one interface, so the check can't pass vacuously.
#
# Deliberately not required, all legitimate:
#
#   * imports the world declares but the .wasm lacks — unused imports are elided
#   * imports the .wasm has but the world does not — builds carry the transitive closure
#   * exports the .wasm has but the world does not — macros add them (#[wstd::http_server])
#
# wasi: is compared at major.minor only: its patch and prerelease tags drift with the
# toolchain (world asks @0.2.0, build carries @0.2.6). Dropping wasi: instead would make
# wasi-only components like log-to-stdout unverifiable. Everything else is compared exactly.
#
# Usage: check-component-matches-world.sh <world.wit> <component.wasm> [display-name]
# Exits 0 when they agree, 1 when they do not, 2 when a tool failed.

set -uo pipefail

world=${1:?usage: check-component-matches-world.sh <world.wit> <component.wasm> [name]}
wasm=${2:?usage: check-component-matches-world.sh <world.wit> <component.wasm> [name]}
name=${3:-$world}

main() {
  world_interfaces=$(get_world_interfaces) || {
    echo "::error::$name: cannot read $world"
    exit 2
  }

  component_interfaces=$(get_component_interfaces) || {
    echo "::error::$name: wasm-tools could not read $wasm"
    exit 2
  }

  declare -A built
  index_versions_by_interface built <<<"$component_interfaces"

  result_code=0
  common=0
  compare_world_against_component built common <<<"$world_interfaces" || result_code=1

  if [ "$common" -eq 0 ]; then
    echo "::error::$name: $world and $wasm have no interface in common, so nothing could be checked"
    echo "::error::$name: this .wasm was almost certainly not built from this world"
    result_code=1
  fi

  exit $result_code
}

# One line per interface, as "<import|export> <interface without version> <version>".
# A version of "-" means the source did not give one.
interfaces() {
  sed -nE 's/^[[:space:]]*(import|export) ([^;]+);.*/\1 \2/p' \
    | awk '{
        dir = $1; ref = $2; ver = "-"
        at = index(ref, "@")
        if (at > 0) { ver = substr(ref, at + 1); ref = substr(ref, 1, at - 1) }
        # wasi: keeps major.minor only; 0.2.6 and 0.2.0-rc.1 both become 0.2
        if (ref ~ /^wasi:/ && match(ver, /^[0-9]+\.[0-9]+/)) ver = substr(ver, RSTART, RLENGTH)
        print dir, ref, ver
      }' \
    | sort -u
}

get_world_interfaces() {
  interfaces <"$world"
}

get_component_interfaces() {
  wasm-tools component wit "$wasm" | interfaces
}

# Example output:
#   built["export betty-blocks-types:crud/crud"]=2.0.0
#   built["import wasi:io/streams"]=0.2
index_versions_by_interface() {
  local -n _map=$1
  local dir ref ver
  while read -r dir ref ver; do
    [ -n "${dir:-}" ] || continue
    _map["$dir $ref"]=$ver
  done
}

# Returns 1 on any disagreement, and counts the interfaces both sides name into $2.
compare_world_against_component() {
  local -n _component=$1 _common=$2
  local dir ref ver key result_code=0
  _common=0

  while read -r dir ref ver; do
    [ -n "${dir:-}" ] || continue
    key="$dir $ref"

    if [ -n "${_component[$key]+set}" ]; then
      _common=$((_common + 1))
      if [ "${_component[$key]}" != "$ver" ]; then
        echo "::error::$name: $dir $ref is @$ver in $world but @${_component[$key]} in the .wasm"
        result_code=1
      fi
    elif [ "$dir" = export ]; then
      echo "::error::$name: $world declares export $ref@$ver but the .wasm does not export it"
      result_code=1
    elif [ "${ref#wasi:}" = "$ref" ]; then
      echo "::error::$name: $world declares import $ref@$ver but the .wasm does not import it"
      result_code=1
    fi
  done

  return $result_code
}

main "$@"
