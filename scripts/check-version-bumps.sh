#!/usr/bin/env bash
#
# CI guard: every package whose code or WIT changed vs the base branch must also
# have its WIT package version bumped, and that bump must be a major bump —
# @X.Y.Z -> @(X+1).0.0 — per docs/decisions/001-major-only-wit-version-bumps.md.
#
# For each package we compare the version in its `package <ns>:<name>@X.Y.Z;`
# declaration against the same file on the base branch. A package is considered
# "changed" when any of these (tracked) files differ from the base:
#   - components/<component>/    : src/**, build.rs, Cargo.toml, wit/** (excl. wit/deps/**)
#   - wit/<shared-pkg>/          : the shared WIT package files (types, actions, ...)
#
# Generated/tooling files never trigger the requirement: wit/deps/ (gitignored),
# target/ (gitignored), wkg.lock, .wash/, tests/, docs, Justfile, etc.
#
# Usage:
#   GITHUB_BASE_REF=main ./scripts/check-version-bumps.sh
#   ./scripts/check-version-bumps.sh <base-ref-or-sha>
set -uo pipefail

BASE_REF="${1:-${GITHUB_BASE_REF:-main}}"

# dev is an integration branch, not a production environment, so overwriting a version that is
# still in development is safe: a changed package may keep the major it already took there. The
# promotion PR compares against main, which still has to show exactly one major step.
REUSE_ALLOWED_ON="dev"

main() {
  # Make the base ref available (no-op if already fetched, e.g. local dev).
  git fetch -q origin "$BASE_REF" 2>/dev/null || true

  if git rev-parse --verify -q "origin/${BASE_REF}^{commit}" >/dev/null; then
    BASE="origin/${BASE_REF}"
  elif git rev-parse --verify -q "${BASE_REF}^{commit}" >/dev/null; then
    BASE="${BASE_REF}"
  else
    echo "::error::base ref '${BASE_REF}' not found (tried origin/${BASE_REF} and ${BASE_REF})"
    exit 1
  fi

  echo "Comparing against base: ${BASE}"

  if [ "$BASE_REF" = "$REUSE_ALLOWED_ON" ]; then
    allow_reuse=true
    echo "Base is ${REUSE_ALLOWED_ON}: a changed package may reuse the version it already has there."
  else
    allow_reuse=false
  fi

  changed="$(git diff --name-only "${BASE}...HEAD")"
  if [ -z "$changed" ]; then
    echo "No changes vs base. Nothing to check."
    exit 0
  fi

  errors=()

  # --- Components: components/**/wit/world.wit (a component root is the parent of wit/) ---
  while IFS= read -r world; do
    [ -z "$world" ] && continue
    dir="$(dirname "$(dirname "$world")")"
    require_bump "$dir" "$world" \
      "^${dir}/(src/|build\.rs$|Cargo\.toml$|wit/)" \
      "^${dir}/wit/deps/"
  done < <(find components -name world.wit -not -path '*/wit/deps/*' | sort)

  # --- Shared WIT packages: wit/<pkg>/*.wit (only .wit changes require a bump, not wkg.toml/lock) ---
  while IFS= read -r witfile; do
    [ -z "$witfile" ] && continue
    dir="$(dirname "$witfile")"
    require_bump "$dir" "$witfile" "^${dir}/[^/]*\.wit$"
  done < <(find wit -mindepth 2 -name '*.wit' -not -path '*/deps/*' | sort)

  echo
  if [ "${#errors[@]}" -gt 0 ]; then
    echo "::error::Version bump required — the following packages changed without a version bump:"
    for e in "${errors[@]}"; do echo "  • ${e}"; done
    exit 1
  fi
  echo "All changed packages have a version bump. ✅"
}

# Extract the version that follows '@' in the first `package ...;` line (reads stdin).
version_of() {
  grep -m1 -E '^package ' | sed -nE 's/^package[^@]*@([^;[:space:]]+).*/\1/p'
}

# require_bump <label> <version-file> <include-ERE> [<exclude-ERE>]
require_bump() {
  local label="$1" vfile="$2" include="$3" exclude="${4:-}"
  local hits base_major want
  hits="$(printf '%s\n' "$changed" | grep -E "$include" || true)"
  [ -n "$exclude" ] && hits="$(printf '%s\n' "$hits" | grep -vE "$exclude" || true)"
  hits="$(printf '%s\n' "$hits" | grep -v '^[[:space:]]*$' || true)"
  [ -z "$hits" ] && return 0 # nothing relevant to this package changed

  local cur base
  cur="$(version_of <"$vfile" 2>/dev/null)"
  base="$(git show "${BASE}:${vfile}" 2>/dev/null | version_of)"

  if [ -z "$base" ]; then
    echo "🆕 ${label}: new package (no version on base) — OK"
    return 0
  fi
  if [ -z "$cur" ]; then
    errors+=("${label} — changed but no '@version' found in ${vfile}")
    echo "❌ ${label}: changed but version is missing/unparseable in ${vfile}"
    return 0
  fi
  if [ "$cur" = "$base" ]; then
    if [ "$allow_reuse" = true ]; then
      echo "♻️  ${label}: changed, version reused @${cur} — allowed on ${BASE_REF}"
      return 0
    fi
    errors+=("${label} — changed but version NOT bumped (still @${cur}); bump the version in ${vfile}")
    echo "❌ ${label}: changed, version still @${cur}"
    return 0
  fi

  # ADR 001: every bump is a major bump — minor and patch stay 0, so each release lands in its
  # own semver compatibility class and two versions of one package never merge in a build.
  base_major="${base%%.*}"
  want="$((base_major + 1)).0.0"
  if [ "$cur" != "$want" ]; then
    errors+=("${label} — @${base} → @${cur}; major-only policy requires @${want} (docs/decisions/001)")
    echo "❌ ${label}: @${base} → @${cur}, expected @${want}"
  else
    echo "✅ ${label}: @${base} → @${cur}"
  fi
}

main "$@"
