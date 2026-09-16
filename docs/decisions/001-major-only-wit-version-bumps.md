# Major-only version bumps for WIT packages and components

## Status

**Accepted.** Applies to every WIT package under `wit/` (namespace `betty-blocks-types`) and
every component under `components/` (namespace `betty-blocks-utilities`).

Enforced by `scripts/check-version-bumps.sh` via the `Version Check` workflow.

First ADR in this repository. It fixes the numbering rule that actions-compiler's
`docs/decisions/001-wit-and-component-versioning.md` and its never-collapse Resolve depend on.

## Context

### The merge rule

`wit-parser` classifies every version into a **semver compatibility class** via
`PackageName::version_compat_track` (`wit-parser/src/lib.rs:207`, byte-identical across
0.225.0 – 0.244.0). The class is the **leftmost non-zero** version component:

| version | class | import name in the core module |
| --- | --- | --- |
| `2.0.0`, `2.1.0`, `2.3.0` | `2` | `cm32p2\|betty-blocks-types:types/types@2` |
| `0.1.0` | `0.1` | `…@0.1` |
| `0.0.3` | `0.0.3` | `…@0.0.3` |
| `2.1.0-rc.1` | `2.1.0-rc.1` (pre-releases are each their own class) | `…@2.1.0-rc.1` |

This is the same convention Cargo and npm use for `^` ranges ("left-most non-zero
major/minor/patch component"), but `wit-parser` implements it independently and attributes it
to the component model, not to either package manager.

When a single build ends up with **two versions of one package in the same class**,
`Resolve::merge_world_imports_based_on_semver` picks the highest and **silently rewrites the
older consumer's imports onto it**. The version is already truncated to the class in the core
module's import name, so the two imports are literally indistinguishable to the encoder;
`wasm-tools component new --merge-imports-based-on-semver false` does **not** restore two
imports — it only changes which version the one truncated name resolves to.

Actions-compiler's Resolve performs a **set-union** over an app's helper dependencies, so
builds carrying two versions of one helper are the normal case, not an edge case.

### Why the rewrite is unsafe

The check performed before that rewrite has three layers, and it is blind where it matters:

| layer | compares | ignores |
| --- | --- | --- |
| A1 types | every type **name** in the older interface exists in the newer | everything about the type's definition — the source carries `FIXME: ideally the types should be "structurally equal"` over an empty body |
| A2 functions | name, kind, parameter count, parameter names, primitive parameter/result kinds | any parameter or result that is a named or anonymous type |
| B core ABI | the older consumer's core `func` type equals the newer function's flattened canonical-ABI signature | anything that flattens identically: field/case names and order, signedness, `string` vs `list<u8>` |

Net effect: **every layout-preserving edit inside a named type is accepted**, and the older
consumer is rebound onto the new memory layout without warning. A renamed record field, a
reordered field, a `u32`→`s32` flip and an added variant case all encode clean.

This is not hypothetical here. `betty-blocks-types:actions` shipped a **breaking change under a
minor bump**: `run-input` gained `jwt: option<string>` in `@2.1.0`. The pair `actions@2.0.0` +
`@2.2.0` fails to encode; the same bytes republished as `@3.0.0` coexist cleanly.

### What else breaks inside one class

Three further failure modes, all gated on the same precondition:

- **Pruned projections fail to merge.** `wasm-tools component wit --importize` emits only the
  types a component actually `use`s. Two pruned projections of the same *additive* package are
  generally not in a subsumption relation, so the merge fails on a type present in **both**
  published versions. Across classes this cannot happen at all.
- **The diagnostic is destroyed.** `wasm-tools` **1.247.0** removed the type-presence check that
  `merge_world_imports_based_on_semver` still depends on while leaving the infallible name
  indexing that followed it. On any `wasm-tools` ≥ 1.247 — including the `wit-parser` 0.251.0
  that `jco` vendors — a missing type becomes a **Rust panic** when another interface `use`s it,
  and a **silent success** when nothing does. No upstream issue mentions it; it is still on
  `main`.
- **Encode-time success is not a version-discipline gate.** 8 of 11 semantics-changing mutations
  measured against the real published history encoded clean.

### Starting state

| `wit/` package | version | next release |
| --- | --- | --- |
| `types` | `2.3.0` | `3.0.0` |
| `actions` | `2.2.0` | `3.0.0` |
| `upload-file` | `3.0.0` | `4.0.0` |
| `auth`, `crud`, `data-api`, `logs-writer`, `pdf-generator`, `smtp` | `2.0.0` | `3.0.0` |

| `components/` package | version | next release |
| --- | --- | --- |
| `http-wrapper` | `2.1.0` | `3.0.0` |
| `upload-file` | `3.0.0` | `4.0.0` |
| `mcp`, `logs-writer`, `test-component`, `auth`, `crud`, `data-api`, `log-to-stdout`, `pdf-generator` | `2.0.0` | `3.0.0` |

No package is currently mid-class with a published sibling, so the policy takes effect from the
next release of each with no migration.

## Decision

**Every published release of a WIT package or component increments the major. Minor and patch
are always `0`.**

```
2.0.0  ->  3.0.0  ->  4.0.0  ->  5.0.0
```

Equivalently, stated as the invariant that actually matters:

> Two versions of one WIT package must never appear in a single build within the same
> compatibility class. Every published release gets a new compatibility class.

### Why major-only rather than staying at `0.x`

`0.1.0 → 0.2.0 → 0.3.0` has the identical compatibility-class property and was considered.
Major-only was chosen because it makes the class **an integer equal to the major**, so:

- the CI assertion is `cur == (base_major + 1).0.0` — no need to implement
  `version_compat_track` anywhere in this repo;
- "are these two versions compatible?" is answered by comparing two integers, by a human or a
  script, with no rule to remember;
- it does not signal "unstable" for packages that are in production.

### Why not keep minor bumps for additive changes

The only thing a compatible bump buys is the merge itself. Concretely: component X built against
`crud@2.0.0`, component Y against `crud@2.1.0`, both pulled into one build by set-union Resolve.

- **Same class:** they merge. X's import is rewritten to `@2.1.0`; one import, **one** crud
  component deployed. That consolidation is the entire benefit.
- **Different classes** (`crud@2`, `crud@3`): no merge. Two imports, **two** crud components
  deployed. Nothing is rewritten.

So the benefit is automatic consolidation, and it is paid for with a rebind that never inspects
the inside of a record. Additive changes are not a safe exception either — the pruned-projection
failure above reproduces on a purely **additive** `2.0.0 → 2.1.0`.

### Enforcement

`scripts/check-version-bumps.sh` already establishes, per package, that a changed package was
bumped (`$cur` vs `$base`). Extend `require_bump` with the shape assertion, after the existing
"changed but not bumped" branch:

```bash
# ADR 001: every bump is a major bump — minor and patch stay 0.
cur_major="${cur%%.*}"
base_major="${base%%.*}"
want="$((base_major + 1)).0.0"
if [ "$cur" != "$want" ]; then
  errors+=("${label} — @${base} → @${cur}; major-only policy requires @${want} (docs/decisions/001)")
  echo "❌ ${label}: @${base} → @${cur}, expected @${want}"
else
  echo "✅ ${label}: @${base} → @${cur}"
fi
```

This runs on both loops already present in the script — `components/**/wit/world.wit` and
`wit/<pkg>/*.wit` — so no new traversal is needed. New packages (no version on base) keep the
existing "🆕 new package — OK" path and may start at any major.

## Consequences

- **No consolidation, ever.** Two consumers on two helper versions means two helper components
  deployed in the workload, where same-class merging would have collapsed them to one. This is
  the honest price of refusing the unchecked rebind, but it is a real operational cost and it
  only grows. It needs a counterweight: a deliberate practice of moving consumers forward onto
  the current major, or versions accumulate indefinitely.
- **Version numbers inflate.** Packages will reach `7.0.0`, `12.0.0`. Cosmetic.
- **The version number stops carrying a compatibility signal.** It no longer distinguishes an
  added field from a removed one. `CHANGELOG.md` and the release notes become the only place
  that says what changed, so they must actually say it.
- **Consumers must opt in to every release.** Nothing is picked up implicitly. This matches
  actions-compiler's "immutable published version + deliberate opt-in" and wash v2's exact-match
  wiring, where a consumer pinned to `@X` already needs an exporter at exactly `@X`.
- **A structural WIT differ is no longer a gate, but stays useful.** With no compatible bumps
  there is nothing for it to fail on. It still answers "was this bump actually breaking?" for
  migration notes — and note that on `wasm-tools` ≥ 1.247 the toolchain catches *less* than
  measured (type removals and renames now encode clean), so a differ's scope is if anything
  wider than before.

## What this does not fix

The policy covers packages **this repo publishes**. It does not cover the rest of the graph:

- `wasi:*` sits at `0.2.x`, so `0.2.12` and `0.2.13` are the **same class** (`@0.2`). Any build
  mixing wasi patch versions still routes through the merge path, and two importized components
  that pruned wasi differently are the pruned-projection failure with a package nobody here
  publishes.
- Therefore, in **actions-compiler**, these remain necessary and must not be dropped on the
  strength of this ADR:
  1. the `wasm-tools` **≤ 1.246** pin, which is load-bearing rather than incidental — above it
     the merge panics instead of reporting;
  2. a `wasm-tools` preflight before `jco componentize`
     (`component embed --dummy` + `component new`), which restores the readable diagnostic and
     catches merges `jco` would silently encode;
  3. the invariant that for any same-class package with more than one version on disk, the
     **highest** version present must be the full registry text, never a pruned projection.

Those belong in actions-compiler's ADR and tests, not here.

## Notes

- The repo's own release version (`CHANGELOG.md`, `.version`, driven by semantic-release from
  conventional commits in `.releaserc.json`) is **independent** of the per-package WIT versions
  edited by hand in the `.wit` files. This ADR governs only the latter; semantic-release
  continues to version the repository normally.
- `upload-file` is the one package that already had two compatibility classes (`2.0.0` and
  `3.0.0`) and is the shape every package now follows.
- Pre-releases are each their own class (`2.1.0-rc.1` merges with nothing, not even `2.1.0`).
  That is a usable lever for staged rollouts, but it is outside this policy — a pre-release is
  not a published release under the rule above.

## Evidence

Measured in `wasmcloud-experimentation-wit-interfaces`, all fabricated, no host or credentials.
Read in this order:

| spike | question | harness |
| --- | --- | --- |
| `semver-class-boundary/` | which version pairs stay distinct? 13 pairs, rule predicted all | `./reproduce.sh` |
| `wit-mutation-matrix/` | what is the merge check, exactly? 27 mutations, three layers | `./matrix.sh` |
| `pruned-merge/` | can two `--importize`d components disagree on a helper version? | `./reproduce.sh all` |
| `jco-error-opacity/` | does the diagnostic survive `jco`? | `./run.sh all` |
| `wit-abi-check/` | which published bumps were breaking? | `./reproduce.sh all` |
