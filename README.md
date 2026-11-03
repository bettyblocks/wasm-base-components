# Wasm base components

Repo that contains the shared components for the Betty Blocks platform. These components can be wasm assembly (WASI) or native plugins that run directly on the server.

The components currently include:

- auth
- crud
- data-api
- upload-file
- pdf-generator
- http-wrapper
- log-to-stdout
- http-mcp
- logs-writer
- live-announcer

These are the names used in the component table in
[`publish-components.yaml`](.github/workflows/publish-components.yaml). The published image name
is not always the same: on ghcr several carry a `-component` suffix (`auth-component`,
`crud-component`, `data-api-component`, `upload-file-component`, `pdf-generator-component`),
and on the Azure registries `http-mcp` is published as `mcp`.

What it doesn't include:

- the actual customer actions
- functions/components that can be imported in Betty Blocks

## Release & Publish

There is one branch: `main`. All PRs go into `main`.

1. Merge a PR into `main`. **Build WASM Components** builds all components and runs
   semantic-release.
2. **Publish WASM Components (after build)** then publishes that build automatically:
   - the components go to ghcr and the dev (edge) registry;
   - the WIT packages go to the dev (edge) registry and the production registry. A WIT version
     has no effect until a step or a service imports that version.
3. Test the components on edge. Then dispatch **Publish WASM Components (production)**. This
   manual step publishes the components of that build to production. An app uses a new
   component at its next compile, thus a person must start this step. The step does not
   rebuild, so production gets the bytes that you tested on edge.

### The workflows, high over

| workflow | runs on | what it does |
|---|---|---|
| **Build WASM Components** (`release.yaml`) | push to `main` that touches something a component is built from | builds every component, runs semantic-release, uploads one `wasm-components` artifact holding **all** of them, and writes its own run id into `.build-run-id` in the release commit |
| **Publish WASM Components (after build)** (`publish.yaml`) | that build finishing, or manual dispatch | selects the build to publish. Publishes the components to ghcr and the dev registry. Publishes the WIT packages to the dev registry and the production registry |
| **Publish WASM Components (production)** (`publish-prod.yaml`) | manual dispatch only | takes a build run id. If the run id is empty, uses the latest release on `main`. Makes sure that the commit of the build is on `main`. Publishes the components of the build to production |
| **Publish components** (`publish-components.yaml`) | called by the two above | all the actual publishing logic, once, identically per destination |
| **CI** / **Version Check** / **Commit Lint** | pull requests | build, test, and lint checks. On a PR, CI builds only the components and WIT packages that the PR changes. A change to a shared file, for example `rust-toolchain.toml`, builds all (`scripts/ci-targets.sh`) |

Publishing always takes a **build run id** as its input, never a commit or a branch. One run id
resolves to exactly one artifact; a commit can have several build runs, so the run id is the only
thing that names a specific set of bytes.

A push to `main` that changes only `docs/`, markdown, `**/tests/`, `.github/` or `scripts/` doesn't
build, so it doesn't publish either — those can't change a component binary. A push is skipped
only when *every* changed file matches, so a commit touching `src/` alongside a test still builds.
Changes to `.github/` and `scripts/` do change how publishing behaves; to exercise them, dispatch
**Publish WASM Components (after build)** with the previous build's run id.

### What gets tagged

Each component gets:

- `X.Y.Z` — the version in its own `wit/world.wit`. A publish never changes an `X.Y.Z` that
  exists.
- `X.Y.Z-<short-sha>` — the same version plus the commit of the build that first published
  `X.Y.Z`.
- `latest` — only on ghcr.

Components come from the build artifact. The WIT packages under `wit/` are built from the
**checkout at that build's commit** instead, and only go to the Azure registries — ghcr gets
components only.

### A publish pushes only new versions

A publish reads the registry before each push. If `X.Y.Z` of a component or a WIT package exists,
the publish skips it. The version check makes each change increase the version. Thus an `X.Y.Z`
that exists already holds the current code, and a new version means a changed package.

The skip is a safety rule: a published version never changes. A change that the version check
does not see, for example a new `rust-toolchain.toml`, cannot overwrite a version in production.

The publish pushes the tags of a component in this sequence: `X.Y.Z-<short-sha>`, `latest`,
`X.Y.Z`. It stops at the first failure. Thus `X.Y.Z` exists only when all tags of the component
exist. When a publish fails halfway, run it again. It pushes the components that it did not
complete.

A `Cargo.lock` change also counts as a change: it changes the bytes of the component, so the
version check requires a new version.

### Publishing a build by hand

Dispatch **Publish WASM Components (production)** with `build_run_id` set to a
**Build WASM Components** run id (the last part of that run's URL). If you leave it empty, the
workflow uses the latest release on `main` (the run id in `.build-run-id`). For edge, dispatch
**Publish WASM Components (after build)** the same way. There, `build_run_id` is required.

A dispatch publishes only the versions that are not in the registry. Use it to release
components to production, or to complete a publish that failed.

GitHub deletes build artifacts after 90 days. Thus, publish a build to production within 90 days
of the merge.

### Rollback

A publish never overwrites a tag, thus a rollback is a new release. Revert the bad change,
increase the version, and merge. Then publish the new build to production.

## Versioning

**Every published release of a WIT package or component must increase its version.** Any
increase — major, minor or patch — is accepted, as long as the version goes up.

```
2.0.0  ->  2.1.0  ->  2.1.1  ->  3.0.0
```

These are the versions written by hand in `wit/<pkg>/*.wit` and `components/<c>/wit/world.wit`.
They are unrelated to the repo's own release version in `CHANGELOG.md`, which semantic-release
keeps bumping normally from conventional commits.

Pick the bump by semver, and treat a breaking change as a major: `wit-parser` merges two
versions of one package that share a compatibility class (the leftmost non-zero component) onto
the newer one, without checking what is inside a record. The details are in
[ADR 001](docs/decisions/001-major-only-wit-version-bumps.md), which is superseded as a policy
but still describes that behaviour.

### No version reuse

Each PR that changes a package must increase the version of that package. This rule also
applies to a follow-up PR of the same feature. Each merge to `main` publishes the WIT packages
to production. Thus a reused version changes a production package in place.

### Enforcement

`scripts/check-version-bumps.sh`, run by the **Version Check** workflow on every PR. It compares
each package whose files changed against the same file on the PR’s base branch:

- changed and not bumped → ❌;
- changed and the version went down → ❌;
- a package with no version on the base is new and may start at any version.

Run it locally with `./scripts/check-version-bumps.sh main`. It diffs `<base>...HEAD`, so it reads **committed**
state — uncommitted edits are invisible to it.

The check gates PRs, not the registry: if a wrong version already got published, correcting it
is a manual deploy — ask the team.

## WIT dependencies

Every package under `wit/` is published to the Betty Blocks registries, and a component takes
each dependency from one of two places:

- **this checkout**, when `wit/<pkg>` declares exactly the version the component asks for —
  which is always true for a version that only exists on your branch, so a WIT change and the
  component that adopts it can land in one PR;
- **the registry** (`bettyblocksdev.azurecr.io`, anonymous pull, no login needed) otherwise —
  a component that stayed on `types@3.0.0` keeps building after `wit/types` moves to `4.0.0`,
  instead of every consumer having to follow the bump.

`scripts/generate-wkg-toml.sh` makes that choice and writes the `wkg.toml` that `wkg` reads. It
runs from `fetch-wit-deps` in every component Justfile (shared via `wit-deps.just`) and from
`publish-wit-packages.sh`, so `just build` is all you need. The generated `wkg.toml` is
gitignored and rewritten on every build — don't commit it, and don't edit it. `wkg.lock`
*is* committed: it pins the registry half of the resolution by digest.

Two consequences worth knowing:

- Run `just build` (or `just fetch-wit-deps`), not a bare `wkg wit fetch` — on its own, wkg
  has neither the overrides nor the registry config and will fail on anything unpublished.
- When a component adopts a new version of a WIT package, everything in its graph has to name
  that same version. `wkg wit fetch` resolves a whole world at once and keys dependencies by
  package name *without* the version, so the first version it meets wins and the rest are
  dropped; the build then fails with `package '...@X.Y.Z' not found. known packages:` naming
  the one that survived. The generator catches it first and names the files that disagree.
  Two versions *can* coexist in principle — a component can import both, and wasmCloud links
  them independently — but only by fetching each with `wkg get <pkg>@<version>`, which this
  repo does not do. Verified in the [WIT interface
  spikes](https://gitlab.betty.services/code/wasmcloud-experimentation-wit-interfaces).

## Local Setup

- install [rust](https://rust-lang.org/tools/install/)
- install [wash](https://wasmcloud.com/docs/installation/)
- install [just](https://github.com/casey/just), 1.27 or newer
- install [bun](https://bun.sh/) (for semantic-release)

## Local Build

- just build

## Local Test

See the [./integration-test](./integration-test) folder

## Repo Layout

- AGENT.md: working rules for coding agents, chiefly the [versioning](#versioning) policy
- Justfile: contains commands to run commands
- components: contains wasm components that are not action steps
- docs/decisions: architecture decision records
- integration-test: Contains the tests to verify that the providers work in wasmcloud
- wit: contains shared WIT interface definitions used by the wasm components
- wit-deps.just: shared `fetch-wit-deps` recipe, imported by every component Justfile
- wkg-config.toml: registry configuration for wkg/wash (see [WIT dependencies](#wit-dependencies))
- .github/workflows: CI/CD pipelines for building, releasing, and publishing
