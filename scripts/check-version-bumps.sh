#!/usr/bin/env bash
#
# CI guard: every package whose code or WIT changed vs the base branch must also
# have its WIT package version bumped.
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
# One WIT-only exemption: a change that does nothing but repoint a `use`/`import`/
# `export` at a new version of *another* package. WIT resolves dependencies by exact
# version, so bumping one package forces every consumer to edit that line; demanding a
# bump there too would cascade a single leaf change across the whole dependency graph
# (and on through each consumer's own consumers). Such a package still describes the
# same interface, so it keeps its version. Anything else in the .wit differing -- a new
# record, a changed signature, or the package's own `package ...@X.Y.Z;` line -- is a
# real change and still requires a bump.
#
# Usage:
#   GITHUB_BASE_REF=main ./scripts/check-version-bumps.sh
#   ./scripts/check-version-bumps.sh <base-ref-or-sha>
set -uo pipefail

BASE_REF="${1:-${GITHUB_BASE_REF:-main}}"

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

# Drop the version tag from every reference to another package, so that a change which
# only repoints a dependency compares equal to the base. The `package ...@X.Y.Z;` line is
# deliberately left intact: that version is the one require_bump checks.
normalize_wit() {
  sed -E '/^[[:space:]]*package[[:space:]]/!s/@[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?//g'
}

# Reads changed paths on stdin. True only when every one of them is a .wit file whose
# sole difference from the base is the version on a reference to another package.
only_dep_version_refs_changed() {
  local f
  while IFS= read -r f; do
    [ -n "$f" ] || continue
    case "$f" in
    *.wit) ;;
    *) return 1 ;; # non-WIT input (src/, Cargo.toml, ...) -- a bump is genuinely required
    esac
    [ -f "$f" ] || return 1                             # deleted
    git show "${BASE}:${f}" >/dev/null 2>&1 || return 1  # newly added
    diff -q <(git show "${BASE}:${f}" | normalize_wit) <(normalize_wit <"$f") >/dev/null || return 1
  done
  return 0
}

# require_bump <label> <version-file> <include-ERE> [<exclude-ERE>]
require_bump() {
  local label="$1" vfile="$2" include="$3" exclude="${4:-}"
  local hits
  hits="$(printf '%s\n' "$changed" | grep -E "$include" || true)"
  [ -n "$exclude" ] && hits="$(printf '%s\n' "$hits" | grep -vE "$exclude" || true)"
  hits="$(printf '%s\n' "$hits" | grep -v '^[[:space:]]*$' || true)"
  [ -z "$hits" ] && return 0 # nothing relevant to this package changed

  if printf '%s\n' "$hits" | only_dep_version_refs_changed; then
    echo "⏭️  ${label}: only dependency version references changed — no bump required"
    return 0
  fi

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
    errors+=("${label} — changed but version NOT bumped (still @${cur}); bump the version in ${vfile}")
    echo "❌ ${label}: changed, version still @${cur}"
  else
    echo "✅ ${label}: changed, version @${base} → @${cur}"
  fi
}

main "$@"
