#!/usr/bin/env bash
#
# Publish the component images of one build to one registry.
#
# Every component is checked before any is pushed, so a failure leaves nothing published — not
# the first half of the table. MODE=check runs the check alone.
#
# Each component gets two tags: X.Y.Z (and latest) from the WIT version, which move with every
# build, and X.Y.Z-<short-sha>, which names the commit the build came from so a deployment can pin
# a tag no later publish moves.
#
# Env:
#   COMPONENTS   required  the component table, one "key|wasm|world|ghcr_name|azure_name"
#                          per line; blank lines and # comments ignored
#   REGISTRY     required  host and namespace for the images, e.g. ghcr.io/bettyblocks
#   NAME_COLUMN  required  ghcr_name | azure_name — which naming scheme this registry uses
#   SOURCE_SHA   required to push; the commit the build was made from, for the pinned tag
#   MODE         optional  check | push | all   (default all)
#   TAG_LATEST   optional  true to also push :latest   (default false)
#   DRY_RUN      optional  true to print pushes instead of making them
#
# To run the check locally against a build you have downloaded, paste the table from
# .github/workflows/publish-components.yaml:
#   COMPONENTS="$table" REGISTRY=ghcr.io/bettyblocks NAME_COLUMN=ghcr_name MODE=check \
#     ./scripts/publish-components.sh

set -uo pipefail

: "${COMPONENTS:?COMPONENTS is required (key|wasm|world|ghcr_name|azure_name per line)}"
: "${REGISTRY:?REGISTRY is required (host and namespace, e.g. ghcr.io/bettyblocks)}"
: "${NAME_COLUMN:?NAME_COLUMN is required (ghcr_name | azure_name)}"
MODE=${MODE:-all}
TAG_LATEST=${TAG_LATEST:-false}
DRY_RUN=${DRY_RUN:-false}
SOURCE_SHA=${SOURCE_SHA:-}
script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

main() {
  validate_mode_and_name_column || exit 2

  if [ "$MODE" != check ]; then
    validate_source_sha || exit 2
    short_sha=${SOURCE_SHA:0:7}
  fi

  component_rows=$(read_component_rows)
  if [ -z "$component_rows" ]; then
    echo "::error::COMPONENTS has no rows"
    exit 2
  fi

  check_every_component_is_tabled "$component_rows" || exit 1

  if [ "$MODE" = check ] || [ "$MODE" = all ]; then
    check_components_match_worlds <<<"$component_rows" || {
      echo "::error::at least one component does not match its world — nothing was pushed"
      exit 1
    }
    echo "Every component matches the world in this checkout."
    [ "$MODE" = check ] && exit 0
  fi

  push_targets=()
  plan_push_targets push_targets <<<"$component_rows" || {
    echo "::error::at least one component could not be published safely — nothing was pushed"
    exit 1
  }

  result_code=0
  summary=$(mktemp)
  push_all_targets push_targets "$summary" || result_code=1

  [ -n "${GITHUB_STEP_SUMMARY:-}" ] && cat "$summary" >>"$GITHUB_STEP_SUMMARY"
  exit $result_code
}

validate_mode_and_name_column() {
  case "$NAME_COLUMN" in
  ghcr_name | azure_name) ;;
  *)
    echo "::error::NAME_COLUMN must be ghcr_name or azure_name, got '$NAME_COLUMN'"
    return 2
    ;;
  esac
  case "$MODE" in
  check | push | all) ;;
  *)
    echo "::error::MODE must be check, push or all, got '$MODE'"
    return 2
    ;;
  esac
}

validate_source_sha() {
  case "$SOURCE_SHA" in
  *[!0-9a-f]* | '')
    echo "::error::SOURCE_SHA must be the build's commit sha, got '$SOURCE_SHA'"
    return 2
    ;;
  esac
  [ "${#SOURCE_SHA}" -ge 7 ] || {
    echo "::error::SOURCE_SHA is too short to pin a tag with: '$SOURCE_SHA'"
    return 2
  }
}

read_component_rows() {
  { grep -vE '^[[:space:]]*(#|$)' <<<"$COMPONENTS" || true; }
}

image_name() {
  local ghcr_name=$1 azure_name=$2
  if [ "$NAME_COLUMN" = ghcr_name ]; then echo "$ghcr_name"; else echo "$azure_name"; fi
}

check_every_component_is_tabled() {
  local rows=$1 world result_code=0

  while read -r world; do
    grep -qE '^package betty-blocks-utilities:' "$world" || continue
    awk -F'|' -v w="$world" '$3 == w { found = 1 } END { exit !found }' <<<"$rows" && continue
    echo "::error::$world declares a betty-blocks-utilities package but is not in the component table"
    result_code=1
  done < <(find components -name world.wit -path '*/wit/*' \
    -not -path '*/deps/*' -not -path '*/test-component/*' | sort)

  return $result_code
}

check_components_match_worlds() {
  local key wasm world ghcr_name azure_name result_code=0

  while IFS='|' read -r key wasm world ghcr_name azure_name; do
    if [ ! -f "$wasm" ]; then
      echo "::error::$key: no .wasm at $wasm — the build bundle is incomplete"
      result_code=1
      continue
    fi
    "$script_dir/check-component-matches-world.sh" "$world" "$wasm" "$key" || result_code=1
  done

  return $result_code
}

plan_push_targets() {
  local -n _targets=$1
  local key wasm world ghcr_name azure_name name version image pinned result_code=0

  while IFS='|' read -r key wasm world ghcr_name azure_name; do
    name=$(image_name "$ghcr_name" "$azure_name")
    version=$(sed -nE 's/^package[^@]*@([^;[:space:]]+).*/\1/p' "$world" | head -1)
    if [ -z "$version" ]; then
      echo "::error::$key: no @version in $world"
      result_code=1
      continue
    fi

    image="$REGISTRY/$name"
    pinned="$version-$short_sha"

    _targets+=("$key|$wasm|$image|$version|$pinned")
  done

  return $result_code
}

push_all_targets() {
  local -n _targets=$1
  local summary=$2
  local target key wasm image version pinned tags tag result_code=0

  for target in "${_targets[@]}"; do
    IFS='|' read -r key wasm image version pinned <<<"$target"

    tags=("$pinned" "$version")
    [ "$TAG_LATEST" = true ] && tags+=("latest")

    for tag in "${tags[@]}"; do
      if [ "$DRY_RUN" = true ]; then
        echo "would push $image:$tag  <-  $wasm"
        echo "- \`$image:$tag\` (dry run)" >>"$summary"
      elif wkg oci push "$image:$tag" "$wasm"; then
        echo "Published $image:$tag"
        echo "- \`$image:$tag\`" >>"$summary"
      else
        echo "::error::$key: failed to push $image:$tag"
        echo "- ~\`$image:$tag\`~ **push failed**" >>"$summary"
        result_code=1
      fi
    done
  done

  return $result_code
}

main "$@"
