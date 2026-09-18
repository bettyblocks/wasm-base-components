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

1. merge to `dev`. **Build WASM Components** builds and runs semantic-release, then
   **Publish WASM Components (dev)** pushes the images to ghcr and the dev registry.
2. merge `dev` into `main`. **Publish WASM Components (production)** republishes that same
   build to the production registry — it does not rebuild, so production runs the bytes that
   were tested on dev.

### The workflows, high over

| workflow | runs on | what it does |
|---|---|---|
| **Build WASM Components** (`release.yaml`) | push to `dev` that touches something a component is built from | builds every component, runs semantic-release, uploads one `wasm-components` artifact holding **all** of them, and writes its own run id into `.build-run-id` in the release commit |
| **Publish WASM Components (dev)** (`publish.yaml`) | that build finishing, or manual dispatch | picks *which* build to publish, nothing more. Fans out to ghcr and the dev registry |
| **Publish WASM Components (production)** (`publish-prod.yaml`) | push to `main`, or manual dispatch | reads the build run id out of `.build-run-id`, checks `main`'s tree still matches that build's commit, then publishes to production |
| **Publish components** (`publish-components.yaml`) | called by the two above | all the actual publishing logic, once, identically per destination |
| **CI** / **Version Check** / **Commit Lint** | pull requests | build, test, and lint checks |

Publishing always takes a **build run id** as its input, never a commit or a branch. One run id
resolves to exactly one artifact; a commit can have several build runs, so the run id is the only
thing that names a specific set of bytes.

A push to `dev` that changes only `docs/`, markdown, `**/tests/`, `.github/` or `scripts/` doesn't
build, so it doesn't publish either — those can't change a component binary. A push is skipped
only when *every* changed file matches, so a commit touching `src/` alongside a test still builds.
Changes to `.github/` and `scripts/` do change how publishing behaves; to exercise them, dispatch
**Publish WASM Components (dev)** with the previous build's run id.

### What gets tagged

Each component gets:

- `X.Y.Z` — the version in its own `wit/world.wit`. Every publish moves it, so a deployment
  pinned to `2.0.0` can get different bytes tomorrow.
- `X.Y.Z-<short-sha>` — the same version plus the build's commit. Publishing any other commit
  leaves it untouched, so pin this one to keep a deployment on what it runs today. It names the
  commit, not the bytes: rebuilding that same commit overwrites it. Use the image digest if you
  need byte-exactness.
- `latest` — only on ghcr.

Components come from the build artifact. The WIT packages under `wit/` are built from the
**checkout at that build's commit** instead, and only go to the Azure registries — ghcr gets
components only.

### Every publish pushes everything

There's no change detection and no skipping. Every component and every tag is pushed on every
publish, even when nothing changed.

That's deliberate: pushing is idempotent, so it's cheap (registries are content-addressed — a
blob that's already there isn't uploaded again) and re-running a publish that failed halfway is
always safe.

Each tag is a separate `wkg oci push` of the same file, because `wkg` pushes a file rather than
pointing a tag at a digest. Only the first uploads the component; the rest find the blob already
present and just write a manifest, so a component is three pushes but one artifact.

Deployments reference `X.Y.Z`, and that tag moves with every publish — so on its own, nothing
records *which build* is currently deployed, and there's no name for the build that worked
yesterday. `X.Y.Z-<short-sha>` is that name: it's the tag a rollback points at, and the one to
pin when a deployment has to stay on what it runs today.

### Republishing an old build, and rollback

Dispatch **Publish WASM Components (production)** with `build_run_id` set to a
**Build WASM Components** run id (the last part of that run's URL). Leave it empty and the build
recorded in `.build-run-id` is used. For dev, dispatch **Publish WASM Components (dev)** the same
way — there `build_run_id` is required.

This is how a rollback works, and it is a real rewrite: `X.Y.Z` and `latest` move **back** to the
older build's bytes, and the WIT packages go back to that commit's WIT. That's the point of a
rollback, but it does mean the mutable tags now point at something older than `main`.

Build artifacts are deleted after 90 days, which bounds how long a `dev` → `main` promotion can
be delayed and how far back a rollback can reach.

### Overwriting a tag

Nothing stops you. Tags are mutable and publishing doesn't check what's already there, so
republishing overwrites whatever the tags currently point at — including `X.Y.Z-<short-sha>` if
you publish a *different* build run of the same commit. Because Rust builds aren't
bit-reproducible, that tag would then hold different bytes than it did before.

Prefer republishing the **original run id** rather than re-running the build for a commit: same
run id means the same artifact, so the push is a genuine no-op. Re-running the build gives you
equivalent-but-different bytes under a tag someone may already have pinned.

## Versioning

**Every published release of a WIT package or component increments the major. Minor and patch
are always `0`.**

```
2.0.0  ->  3.0.0  ->  4.0.0  ->  5.0.0
```

These are the versions written by hand in `wit/<pkg>/*.wit` and `components/<c>/wit/world.wit`.
They are unrelated to the repo's own release version in `CHANGELOG.md`, which semantic-release
keeps bumping normally from conventional commits.

The reasoning — what `wit-parser` merges inside one compatibility class, and why that rewrite is
unsafe — is in [ADR 001](docs/decisions/001-major-only-wit-version-bumps.md).

### Iterating on dev

`dev` tags are mutable — every publish overwrites `X.Y.Z` in place — so a change still in flight
does not need a new major per PR. Take the next major **once**, then keep reusing it:

| PR | package version | verdict |
|---|---|---|
| first feature PR into `dev` | `2.0.0` → `3.0.0` | ✅ the one bump |
| follow-up PRs into `dev` | stays `3.0.0` | ♻️ reuse, allowed |
| promotion PR `dev` → `main` | `2.0.0` → `3.0.0` vs `main` | ✅ exactly one major step |

Reuse needs the PR’s base to be `dev`, and a bump made there still has to be `@(X+1).0.0`. Bump
twice on `dev` and the promotion to `main` is blocked — collapse the versions back to one major
first. Because a reused tag points at different bytes than it did yesterday, pin
`X.Y.Z-<short-sha>` for anything that has to stay put — see
[What gets tagged](#what-gets-tagged).

### Enforcement

`scripts/check-version-bumps.sh`, run by the **Version Check** workflow on every PR. It compares
each package whose files changed against the same file on the PR’s base branch:

- changed and not bumped → ❌, unless the base is `dev`, where it is ♻️ reuse;
- changed and bumped to anything but `@(X+1).0.0` → ❌, on every base;
- a package with no version on the base is new and may start at any major.

Run it locally with the base you intend to target: `./scripts/check-version-bumps.sh dev` or
`./scripts/check-version-bumps.sh main`. It diffs `<base>...HEAD`, so it reads **committed**
state — uncommitted edits are invisible to it.

The check gates PRs, not the registry: if a wrong major already got published, correcting it is
a manual deploy — ask the team.

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
