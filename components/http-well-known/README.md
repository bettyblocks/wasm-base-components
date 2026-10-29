# Well-known component

Serves the OAuth discovery documents (`/.well-known/...`) of one Betty Blocks application, so MCP clients can find out how to log into it. It runs in the mcp workload of the application and needs no backing store: the documents come from the workload config, pre-rendered by actions-compiler on publish.

CORS is left to the wasmCloud runner.

## Endpoints

| Path | Returns |
|---|---|
| `GET /.well-known/oauth-protected-resource/api/runtime/{app_uuid}/authentication_profile/{profile_id}` | `oauth-protected-resource` document |
| `GET /.well-known/oauth-authorization-server/api/runtime/{app_uuid}/authentication_profile/{profile_id}` | `oauth-authorization-server` document |

| Status | When |
|---|---|
| 200 | Document found (`application/json`) |
| 404 | Other application, unknown profile, missing document, invalid id or unknown path |
| 405 | Method other than `GET` |
| 500 | Missing or invalid configuration |

Ids may only contain `[A-Za-z0-9_-]` (max 64 chars). `app_uuid` must equal the `APPLICATION_ID` environment variable of the component.

## Configuration

| Source | Key | Purpose |
|---|---|---|
| environment | `APPLICATION_ID` | The application this component serves |
| wasi:config | `well_known` | The documents, as JSON |

```json
{
  "profiles": {
    "<profile_id>": {
      "oauth-protected-resource": { "...": "..." },
      "oauth-authorization-server": { "...": "..." }
    }
  }
}
```

## Build & test

From the component directory: `just test`, `just build`, `just clippy`, `just formatting --check`.
