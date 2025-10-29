# GitHub Packages Integration

This project publishes WASM components to **GitHub Packages** (GitHub Container Registry) at `ghcr.io`.

## Published Packages

All components are published under: `ghcr.io/betty-blocks/`

Available packages:
- `ghcr.io/betty-blocks/data-api`
- `ghcr.io/betty-blocks/key-vault`
- `ghcr.io/betty-blocks/smtp-provider`

## Automatic Publishing

When a release is created via semantic-release:
1. The `publish.yaml` workflow is automatically triggered
2. Components are authenticated using `GITHUB_TOKEN`
3. All built artifacts are pushed with both version and `latest` tags
4. Packages are made public and visible in the organization

## Using Published Packages

### Pull a Package

```bash
# Latest version
wash pull ghcr.io/betty-blocks/data-api:latest

# Specific version
wash pull ghcr.io/betty-blocks/data-api:1.2.3
```

### In wadm.yaml

```yaml
spec:
  components:
    - name: data-api
      type: capability
      properties:
        image: ghcr.io/betty-blocks/data-api:1.2.3
```

## Manual Publishing

### Prerequisites

You need a GitHub Personal Access Token (PAT) with package permissions:

1. **Create PAT**:
   - Go to: Settings → Developer settings → Personal access tokens → Tokens (classic)
   - Click "Generate new token (classic)"
   - Select scopes:
     - ✅ `write:packages` - Upload packages to GitHub Package Registry
     - ✅ `read:packages` - Download packages from GitHub Package Registry
     - ✅ `delete:packages` - Delete packages from GitHub Package Registry (optional)
   - Generate and copy the token

2. **Set your GitHub credentials as environment variables**:
   ```bash
   # Using just command (interactive login)
   just login
   
   # Or set as environment variables for inline authentication
   export GITHUB_USER="your_github_username"
   export GITHUB_TOKEN="your_token_here"
   ```

### Publish Commands

```bash
# Build components first
just build

# Set credentials (if not already logged in)
export GITHUB_USER="your_username"
export GITHUB_TOKEN="your_pat_token"

# Publish specific version
just upload 1.2.3

# Publish and tag as latest
just upload-latest 1.2.3

# Override repository owner (if different)
REPO_OWNER=my-org just upload 1.2.3

# Override registry (e.g., to use Docker Hub instead)
REGISTRY=docker.io REPO_OWNER=myuser just upload 1.2.3
```

**Note**: The `just upload` commands will use credentials from:
1. The `just login` session (if you ran it), OR
2. Environment variables `GITHUB_USER` and `GITHUB_TOKEN`

## Package Visibility

Packages can be:
- **Public**: Anyone can pull without authentication
- **Private**: Requires authentication to pull

To make packages public:
1. Go to the package page: `https://github.com/orgs/betty-blocks/packages/container/PACKAGE_NAME`
2. Click "Package settings"
3. Scroll to "Danger Zone"
4. Click "Change visibility" → "Public"

## Workflow Permissions

The `publish.yaml` workflow requires these permissions:

```yaml
permissions:
  contents: read      # Read repository content
  packages: write     # Push packages to GitHub Container Registry
```

These are automatically provided via `GITHUB_TOKEN` in GitHub Actions.

## Troubleshooting

### Authentication Failed

**Error**: `unauthorized: authentication required`

**Solution**:
- Ensure you're logged in: `just login`
- Verify your PAT has `write:packages` scope
- Check token hasn't expired

### Package Name Must Be Lowercase

**Error**: `invalid reference format` or `name must be lowercase`

**Solution**:
- GitHub Container Registry requires lowercase names
- The workflow automatically converts `REPO_OWNER` to lowercase
- If manually pushing, ensure: `ghcr.io/betty-blocks/...` (all lowercase)

### Permission Denied in Workflow

**Error**: `403 Forbidden` in GitHub Actions

**Solution**:
- Check workflow has `packages: write` permission
- Verify `GITHUB_TOKEN` is being used correctly
- Ensure the repository has package permissions enabled

### Package Not Visible

**Issue**: Package published but not visible in organization

**Solution**:
- Check package settings for visibility
- Link package to repository:
  1. Go to package settings
  2. Under "Danger Zone" → "Connect repository"
  3. Select this repository

## Best Practices

1. **Version Tags**: Always publish with semantic version tags (e.g., `1.2.3`)
2. **Latest Tag**: Also tag stable releases as `latest`
3. **Testing**: Test components locally before publishing
4. **Documentation**: Update README when adding new components
5. **Security**: Never commit GitHub PAT tokens to the repository

## Cost and Limits

GitHub Packages is free for public repositories with these limits:
- **Storage**: 500 MB free
- **Data transfer**: 1 GB/month free
- **Public packages**: Unlimited bandwidth

For private repositories, see [GitHub Packages pricing](https://docs.github.com/en/billing/managing-billing-for-github-packages/about-billing-for-github-packages).

## References

- [GitHub Packages Documentation](https://docs.github.com/en/packages)
- [Container Registry Documentation](https://docs.github.com/en/packages/working-with-a-github-packages-registry/working-with-the-container-registry)
- [Authenticating to GitHub Packages](https://docs.github.com/en/packages/working-with-a-github-packages-registry/working-with-the-container-registry#authenticating-to-the-container-registry)

