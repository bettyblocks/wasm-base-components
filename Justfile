# note that wash providers break when using a rust workspace for some reason

# Get version from .version file or use default
VERSION := `cat .version 2>/dev/null || echo "0.1.0"`
REGISTRY := env_var_or_default("REGISTRY", "ghcr.io")
REPO_OWNER := env_var_or_default("REPO_OWNER", "bettyblocks")

build:
  wash build --config-path providers/data-api
  wash build --config-path providers/key-vault
  wash build --config-path providers/smtp-provider
  wash build --config-path helper/http-wrapper
  wash build --config-path helper/data-api/component
  just --working-directory helper/log-to-stdout --justfile helper/log-to-stdout/Justfile build

build-test:
  wash build --config-path integration-test/components/fetcher

deploy:
  wash app deploy --replace wadm.yaml

# Upload with dynamic version to GitHub Packages (requires login or GITHUB_USER/GITHUB_TOKEN env vars)
upload VERSION=VERSION:
  @echo "Uploading version {{VERSION}} to {{REGISTRY}}/{{REPO_OWNER}}"
  @echo "Uploading providers..."
  wash push {{REGISTRY}}/{{REPO_OWNER}}/data-api:{{VERSION}} ./providers/data-api/build/data-api.par.gz
  wash push {{REGISTRY}}/{{REPO_OWNER}}/key-vault:{{VERSION}} ./providers/key-vault/build/key-vault.par.gz
  wash push {{REGISTRY}}/{{REPO_OWNER}}/smtp-provider:{{VERSION}} ./providers/smtp-provider/build/smtp-provider.par.gz
  @echo "Uploading WASM components..."
  wash push {{REGISTRY}}/{{REPO_OWNER}}/crud-component:{{VERSION}} ./helper/data-api/component/build/crud_component.wasm
  wash push {{REGISTRY}}/{{REPO_OWNER}}/http-wrapper:{{VERSION}} ./helper/http-wrapper/build/http_wrapper.wasm
  wash push {{REGISTRY}}/{{REPO_OWNER}}/log-to-stdout:{{VERSION}} ./target/wasm32-wasip2/release/log_to_stdout.wasm

# Upload and tag as latest (requires login or GITHUB_USER/GITHUB_TOKEN env vars)
upload-latest VERSION=VERSION:
  just upload {{VERSION}}
  @echo "Tagging as latest..."
  wash push --allow-latest {{REGISTRY}}/{{REPO_OWNER}}/data-api:latest ./providers/data-api/build/data-api.par.gz
  wash push --allow-latest {{REGISTRY}}/{{REPO_OWNER}}/key-vault:latest ./providers/key-vault/build/key-vault.par.gz
  wash push --allow-latest {{REGISTRY}}/{{REPO_OWNER}}/smtp-provider:latest ./providers/smtp-provider/build/smtp-provider.par.gz
  wash push --allow-latest {{REGISTRY}}/{{REPO_OWNER}}/crud-component:latest ./helper/data-api/component/build/crud_component.wasm
  wash push --allow-latest {{REGISTRY}}/{{REPO_OWNER}}/http-wrapper:latest ./helper/http-wrapper/build/http_wrapper.wasm
  wash push --allow-latest {{REGISTRY}}/{{REPO_OWNER}}/log-to-stdout:latest ./target/wasm32-wasip2/release/log_to_stdout.wasm

# Login to GitHub Container Registry
login:
  @echo "Logging in to {{REGISTRY}}..."
  @echo "Use your GitHub personal access token (PAT) with read:packages and write:packages scopes"
  wash reg login {{REGISTRY}}

# Show current version
version:
  @echo "Current version: {{VERSION}}"
