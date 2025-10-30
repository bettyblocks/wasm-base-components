# Semantic Release Setup

This project uses [semantic-release](https://github.com/semantic-release/semantic-release) for automated versioning and releases.

## How It Works

1. **Commit to main**: Push commits using [Conventional Commits](./COMMIT_CONVENTION.md) format
2. **Build & Analyze**: GitHub Actions builds WASM components and analyzes commits
3. **Version & Release**: Semantic-release determines the next version and creates a GitHub release
4. **Publish**: The publish workflow automatically pushes artifacts to the registry

## Workflow Overview

### 1. Release Workflow (`release.yaml`)
- **Trigger**: Push to `main` branch
- **Actions**:
  - Build all WASM components
  - Run semantic-release to determine version
  - Create GitHub release with built artifacts
  - Upload artifacts for later use

### 2. Publish Workflow (`publish.yaml`)
- **Trigger**: GitHub release published OR manual dispatch
- **Actions**:
  - Download release artifacts
  - Push to container registry with version tag
  - Also tag as `latest`
  - Generate summary report

## Configuration Files

### `.releaserc.json`
Main semantic-release configuration:
- **Commit Analyzer**: Determines version bump from commits
- **Release Notes**: Generates changelog from commits
- **Changelog**: Updates `CHANGELOG.md`
- **GitHub**: Creates releases and uploads artifacts
- **Git**: Commits changelog back to repo

### `package.json`
Defines semantic-release dependencies and scripts.

### `Justfile`
Enhanced with dynamic versioning:
```bash
# Show current version
just version

# Upload with specific version
just upload 1.2.3

# Upload and tag as latest
just upload-latest 1.2.3
```

## Version Bumping Rules

| Commit Type | Version Bump | Example |
|-------------|--------------|---------|
| `feat:` | MINOR (0.x.0) | `feat: add new API endpoint` |
| `fix:` | PATCH (0.0.x) | `fix: resolve connection timeout` |
| `feat!:` or `BREAKING CHANGE:` | MAJOR (x.0.0) | `feat!: remove old API` |
| `docs:`, `chore:`, etc. | None | No release created |

## Manual Operations

### Trigger Manual Publish
You can manually publish a specific version:

1. Go to GitHub Actions → Publish WASM Components
2. Click "Run workflow"
3. Enter the version number (e.g., `1.2.3`)
4. Optionally specify a different registry

### Skip CI
To push commits without triggering a release:
```bash
git commit -m "chore: update docs [skip ci]"
```

## Artifacts Published

Each release includes:
- `data-api-vX.Y.Z.par.gz` - Data API provider
- `key-vault-vX.Y.Z.par.gz` - Key Vault provider
- `smtp-provider-vX.Y.Z.par.gz` - SMTP provider
- `crud-component-vX.Y.Z.wasm` - CRUD component
- `send-mail-component-vX.Y.Z.wasm` - Send mail component
- Signed versions (`*_s.wasm`)

## Registry Publishing

Components are published to: **GitHub Packages** (GitHub Container Registry)

Registry URL: `ghcr.io/bettyblocks`

Each component is published with two tags:
- Version-specific: `ghcr.io/bettyblocks/data-api:1.2.3`
- Latest: `ghcr.io/bettyblocks/data-api:latest`

### Accessing Published Packages

Packages are available at:
- View in GitHub: `https://github.com/orgs/bettyblocks/packages`
- Pull command: `wash pull ghcr.io/bettyblocks/COMPONENT_NAME:VERSION`

### Authentication for Manual Publishing

To manually publish, you need a GitHub Personal Access Token (PAT):

1. Go to GitHub Settings → Developer settings → Personal access tokens → Tokens (classic)
2. Generate new token with scopes: `write:packages`, `read:packages`
3. Login to ghcr.io:
   ```bash
   just login
   # Or manually:
   wash reg login ghcr.io --username YOUR_GITHUB_USERNAME
   ```

## Troubleshooting

### No Release Created
- Check commit messages follow Conventional Commits format
- Ensure commits include `feat:`, `fix:`, or breaking changes
- `docs:`, `chore:`, `refactor:` don't trigger releases

### Build Artifacts Missing
- Verify build step completed successfully in release workflow
- Check that components are in expected `build/` directories

### Publish Failed
- Verify `GITHUB_TOKEN` has `packages: write` permission (should be automatic)
- Check that the workflow has correct permissions in the job definition
- Ensure artifacts were uploaded to the release
- Check that repository owner name is lowercase (ghcr.io requirement)

## First Release

To create your first release:

1. Ensure you have semantic-release set up (already done!)
2. Push a commit with `feat:` or `fix:`:
   ```bash
   git commit -m "feat: initial release of WASM components"
   git push origin main
   ```
3. Watch GitHub Actions create the first release (v1.0.0)

## Examples

### Creating a Feature Release
```bash
git commit -m "feat(data-api): add Redis caching support"
git push origin main
# Creates release v1.1.0
```

### Creating a Patch Release
```bash
git commit -m "fix(smtp-provider): handle timeout errors gracefully"
git push origin main
# Creates release v1.1.1
```

### Creating a Breaking Change
```bash
git commit -m "feat(key-vault)!: change encryption algorithm

BREAKING CHANGE: Previous secrets encrypted with old algorithm
must be migrated using the provided migration script."
git push origin main
# Creates release v2.0.0
```

