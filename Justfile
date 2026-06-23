# note that wash providers break when using a rust workspace for some reason

# Get version from .version file or use default
VERSION := `cat .version 2>/dev/null || echo "0.1.0"`
REGISTRY := env_var_or_default("REGISTRY", "ghcr.io")
REPO_OWNER := env_var_or_default("REPO_OWNER", "bettyblocks")

build-all:
    #!/usr/bin/env bash
    set -euo pipefail
    find . -name Justfile -not -path "./Justfile" -mindepth 2 -print0 | while IFS= read -r -d '' justfile; do
        dir=$(dirname "$justfile")
        echo "--- Running builds in $dir ---"
        just --working-directory "$dir" --justfile "$justfile" build
    done

build: build-all

deploy env version:
  cd deploy && bun install
  cd deploy && bun run publish {{env}} {{version}}

test-all:
    #!/usr/bin/env bash
    set -euo pipefail
    find . -name Justfile -not -path "./Justfile" -mindepth 2 -print0 | while IFS= read -r -d '' justfile; do
        dir=$(dirname "$justfile")
        echo "--- Running tests in $dir ---"
        just --working-directory "$dir" --justfile "$justfile" test
    done

lint-all:
    #!/usr/bin/env bash
    set -euo pipefail
    find . -name Justfile -not -path "./Justfile" -mindepth 2 -print0 | while IFS= read -r -d '' justfile; do
        dir=$(dirname "$justfile")
        echo "--- Running linter in $dir ---"
        just --working-directory "$dir" --justfile "$justfile" lint
    done

clippy-all:
    #!/usr/bin/env bash
    set -euo pipefail
    find . -name Justfile -not -path "./Justfile" -mindepth 2 -print0 | while IFS= read -r -d '' justfile; do
        dir=$(dirname "$justfile")
        echo "--- Running clippy in $dir ---"
        just --working-directory "$dir" --justfile "$justfile" clippy
    done

format-all:
    #!/usr/bin/env bash
    set -euo pipefail
    find . -name Justfile -not -path "./Justfile" -mindepth 2 -print0 | while IFS= read -r -d '' justfile; do
        dir=$(dirname "$justfile")
        echo "--- Running formatter in $dir ---"
        just --working-directory "$dir" --justfile "$justfile" formatting
    done

format-ci:
    #!/usr/bin/env bash
    set -euo pipefail
    find . -name Justfile -not -path "./Justfile" -mindepth 2 -print0 | while IFS= read -r -d '' justfile; do
        dir=$(dirname "$justfile")
        echo "--- Running formatter in $dir ---"
        just --working-directory "$dir" --justfile "$justfile" formatting --check
    done

integration-test:
    cd components/http-mcp && cargo test --test integration_test_mcp -- --nocapture

all: test-all lint-all clippy-all format-all
