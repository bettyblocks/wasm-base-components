# Jwt Auth Component 

This component provides JWT validation services used by other components (for example, the HTTP MCP component). It validates tokens signed with **RS256** (RSA public key) and **HS512** (shared secret) and returns structured `Claims` on success.

## Features

- Supports RS256 and HS512 signed JWTs
- Verifies `exp`, `nbf`, `iss` (issuer) and `aud` (audience)
- Returns a `Claims` object with application/user metadata

## Environment variables

- `JWT_PUBLIC_KEY` — PEM encoded RSA public key for RS256 verification
- `JWT_SECRET` — shared secret for HS512 verification (alternative to `JWT_PUBLIC_KEY`)
- `JWT_ISSUER` — expected token issuer
- `JWT_AUDIENCE` — expected token audience

## API

The exported interface provides:

- `validate_token(token: String) -> Result<Claims, AuthError>`

`validate_token` checks the token header to determine the algorithm and runs the appropriate validation path. It returns `AuthError` variants for missing/invalid config, malformed tokens, unsupported algorithms, or validation failures.

## Example

Set required environment variables and call the guest export with a `Bearer` token string:

```bash
export JWT_PUBLIC_KEY="$(cat public.pem)"
export JWT_ISSUER=Joken
export JWT_AUDIENCE=Joken
```

## Build & test

From the component directory:

- Run unit tests: `cargo test`
- Build the WASM: `wash build` (requires `wash` and the wasm toolchain)
- Format & lint: `cargo fmt` and `cargo clippy` (or `just lint-all` from repo root)


## Notes

- The validation logic enforces token expiration and not-before checks and tolerates a small leeway for clock skew.
- Only RS256 and HS512 are supported currently. Passing tokens signed with other algorithms will return `UnsupportedAlgorithm`.
