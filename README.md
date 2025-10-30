# Wasm base components

Repo that contains the shared components for the Betty Blocks platform. These components can be wasm assembly (WASI) or native plugins that run directly on the server.

The components currenly include:
- crud-component
- data-api
- key-vault
- http-wrapper
- types
- http-server

What it doesn't include:
- the actual customer actions
- functions/components that can be imported in Betty Blocks

### note that wash providers break when using a rust workspace for some reason

## Setup

- install [rust](https://rust-lang.org/tools/install/)
- install [wash](https://wasmcloud.com/docs/installation/)
- install [just](https://github.com/casey/just)
- install [bun](https://bun.sh/) (for semantic-release)

## Build

- just build

## Release & Deployment

This project uses **semantic-release** for automated versioning and publishes to **GitHub Packages**.

📚 Documentation:
- [Semantic Release Setup](./.github/SEMANTIC_RELEASE.md) - How releases work
- [GitHub Packages Guide](./.github/GITHUB_PACKAGES.md) - Publishing & authentication
- [Commit Conventions](./.github/COMMIT_CONVENTION.md) - How to write commits (deleted, but referenced)

### Quick Start
```bash
# Make changes and commit using conventional commits
git commit -m "feat: add new feature"
git push origin main

# Semantic-release automatically:
# 1. Determines version bump
# 2. Creates GitHub release
# 3. Publishes to registry
```

### Manual Publishing

First, login to GitHub Container Registry:
```bash
# Login to ghcr.io (you'll need a GitHub Personal Access Token)
just login
```

Then publish:
```bash
# Publish specific version
just upload 1.2.3

# Publish and tag as latest
just upload-latest 1.2.3

# Check current version
just version
```

Components are published to: `ghcr.io/bettyblocks/COMPONENT_NAME`

## Test locally

See the [./integration-test](./integration-test) folder

## Layout

- Justfile: contains commands to run commands
- providers: contains code that needs state and/or os-level access
- integration-test: Contains the tests to verify that the providers work in wasmcloud
- .github/workflows: CI/CD pipelines for building, releasing, and publishing

