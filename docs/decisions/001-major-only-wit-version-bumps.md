# Major-only version bumps for WIT packages and components

## Status

**Accepted.** Applies to every WIT package under `wit/` (namespace `betty-blocks-types`) and
every component under `components/` (namespace `betty-blocks-utilities`).

Enforced by `scripts/check-version-bumps.sh` via the `Version Check` workflow.

First ADR in this repository. It fixes the numbering rule that actions-compiler's
`docs/decisions/001-wit-and-component-versioning.md` and its never-collapse Resolve depend on.

## Context

Everything below was measured in the spike repo
[wasmcloud-experimentation-wit-interfaces](https://gitlab.betty.services/code/wasmcloud-experimentation-wit-interfaces);
[Evidence](#evidence) maps each claim to the spike that established it.

### The merge rule

`wit-parser` buckets every version into a **semver compatibility class** — the leftmost non-zero
component, so `2.0.0`, `2.1.0` and `2.3.0` are all class `2`. When one build ends up with two
versions of a package in the same class, `Resolve::merge_world_imports_based_on_semver` picks the
highest and silently rewrites the older consumer’s imports onto it. Turning the flag off
(`--merge-imports-based-on-semver false`) does not give you two imports back — the class is
already baked into the import name, so it only changes which version that one name resolves to.

Actions-compiler’s Resolve set-unions an app’s helper dependencies, so builds carrying two
versions of one helper are the normal case, not an edge case.

### Why the rewrite is unsafe

The check run before the rewrite compares type **names**, function signatures and the flattened
core ABI — never the inside of a named type. Every layout-preserving edit therefore passes: a
renamed record field, a reordered field, a `u32`→`s32` flip. The older consumer is rebound onto
the new memory layout without warning. 8 of 11 semantics-changing mutations measured against the
real published history encoded clean (`wit-mutation-matrix/`).

Not hypothetical here: `betty-blocks-types:actions` shipped a breaking change under a minor bump
(`run-input` gained `jwt: option<string>` in `@2.1.0`). The pair `actions@2.0.0` + `@2.2.0` fails
to encode; the same bytes republished as `@3.0.0` coexist cleanly.

### What else breaks inside one class

- **Pruned projections fail to merge.** `wasm-tools component wit --importize` emits only the
  types a component actually `use`s. Two pruned projections of the same *additive* package are
  generally not in a subsumption relation, so the merge fails on a type present in **both**
  published versions. Across classes this cannot happen at all.
- **The diagnostic is destroyed on `wasm-tools` ≥ 1.247**, where a missing type becomes a Rust
  panic or a silent success instead of an error — which is what makes actions-compiler’s
  `wasm-tools ≤ 1.246` pin load-bearing rather than incidental.

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

Enforced in CI by `scripts/check-version-bumps.sh`, run by the **Version Check** workflow on
every PR. A package with no version on the base branch is new and may start at any major.

## Consequences

**No consolidation, ever.** Two consumers on two helper versions means two helper components
deployed, where same-class merging would have collapsed them to one. That is the price of
refusing the unchecked rebind, and it grows unless consumers are deliberately moved forward onto
the current major.

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
