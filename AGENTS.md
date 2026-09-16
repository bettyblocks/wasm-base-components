# Agent instructions

## Version bumps are major-only

**Every published release of a WIT package or component increments the major. Minor and patch
are always `0`.**

```
2.0.0  ->  3.0.0  ->  4.0.0  ->  5.0.0
```

This is not a style preference. `wit-parser` buckets versions into a compatibility class (the
leftmost non-zero component), and two versions of one package in the same class within a single
build get silently merged — the older consumer's imports are rewritten onto the newer version
without any check of what is inside a record. Renamed fields, reordered fields and signedness
flips all pass. A new major puts every release in its own class, so the merge never has a pair
to act on. Full reasoning: [ADR 001](docs/decisions/001-major-only-wit-version-bumps.md).

Applies to the hand-written `package …@X.Y.Z;` line in:

- `wit/<pkg>/*.wit` — namespace `betty-blocks-types`
- `components/<component>/wit/world.wit` — namespace `betty-blocks-utilities`

It does **not** apply to the repo's own release version (`CHANGELOG.md`, `.version`), which
semantic-release manages from conventional commits. Leave that alone.

### The dev exception

`dev` tags are mutable; every publish overwrites `X.Y.Z` in place. So while a change is still
being iterated on, a package may **reuse** the major it already took on `dev` rather than
burning a new one per PR.

- PR based on `dev` → reusing the current version is allowed.
- PR `dev` → `main` → the bump is mandatory, measured against `main`.

Take the next major **once**, then reuse it until the change is promoted. Do not bump a second
time on `dev`: `2.0.0` → `3.0.0` → `4.0.0` passes each dev PR and then blocks the promotion,
because `main` would be asked to jump two majors. If you find yourself about to bump a package
whose version already leads `main` by one major, reuse it instead.

Reuse never relaxes the shape — a bump made on `dev` must still be `@(X+1).0.0`. `3.0.0` →
`3.0.1` is rejected everywhere.

## When you change a WIT package

1. Bump the package's own `package …@X.Y.Z;` to the next major (or reuse, per above).
2. **Repoint every reference to it** — `use`/`import`/`export` lines naming that package, in
   both `wit/` and `components/`. Everything that reaches one package must name the same
   version of it; `wkg` keys dependencies by name without the version, so the first version it
   meets wins and the rest are silently dropped.
3. If the package is exported by a component, that component's world changed — bump it too.
4. A package whose dependency edges changed has changed, even when its own types did not.
   Republishing the same version with different content is not allowed off `dev`.

`scripts/generate-wkg-toml.sh` catches disagreements and names the files that conflict. It also
decides where each dependency comes from: the local checkout when `wit/<pkg>` declares exactly
the version asked for, the registry otherwise.

## Files not to hand-edit

- `wkg.toml` — generated on every build and gitignored. Never commit or edit it.
- `wkg.lock` — committed, but rewritten by `wkg`. To refresh it, run the build rather than
  editing: `wkg wit build --wit-dir .` in a `wit/<pkg>` directory, or `just build` (which runs
  `wkg wit fetch`) in a component directory.

## Verify before finishing

```sh
./scripts/check-version-bumps.sh dev     # or main, matching the PR's base
just build-all
```

`check-version-bumps.sh` diffs `<base>...HEAD`, so it only sees **committed** state —
uncommitted edits are invisible to it and it will report nothing.
