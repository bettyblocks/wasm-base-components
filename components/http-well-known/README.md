# Well-known component

Serves the OAuth discovery documents (`/.well-known/...`) of one Betty Blocks application, so MCP clients can find out how to log into it. It runs in a workload of its own (a workload can only have one http component, so it can not share the one of `http-mcp`) and needs no backing store: the documents come from the workload config, pre-rendered by actions-compiler on publish.

CORS is left to the wasmCloud runner.

## Endpoints

| Path | Returns |
|---|---|
| `GET /.well-known/oauth-protected-resource/api/mcp/{mcp_id}` | `oauth-protected-resource` document (RFC 9728) of one mcp server. Its `resource` is the url of that server, so every server has its own document, also when servers share an authentication profile |
| `GET /.well-known/oauth-authorization-server/api/runtime/{app_uuid}/authentication_profile/{profile_id}` | `oauth-authorization-server` document (RFC 8414) of one authentication profile |

| Status | When |
|---|---|
| 200 | Document found (`application/json`) |
| 404 | Unknown mcp server or profile, other application, invalid id or unknown path |
| 405 | Method other than `GET` |
| 500 | Missing or invalid configuration |

For the authorization server, the app and profile id may only contain `[A-Za-z0-9_-]` (max 64 chars) and `app_uuid` must equal the `APPLICATION_ID` environment variable of the component.

## Configuration

| Source | Key | Purpose |
|---|---|---|
| environment | `APPLICATION_ID` | The application this component serves |
| wasi:config | `mcp_servers` | The mcp config of the application, shared with `http-mcp`. This component reads its `well-known` part |

```json
{
  "mcp-servers": ["..."],
  "well-known": {
    "oauth-protected-resource": {
      "<mcp_id>": { "resource": "https://<host>/api/mcp/<mcp_id>", "...": "..." }
    },
    "oauth-authorization-server": {
      "<profile_id>": { "issuer": "...", "...": "..." }
    }
  }
}
```

## Build & test

From the component directory: `just test`, `just build`, `just clippy`, `just formatting --check`.
