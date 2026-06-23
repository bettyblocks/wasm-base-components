# Http-MCP component 

Lightweight HTTP gateway that exposes a Model Context Protocol (MCP) JSON-RPC 2.0 API. It validates incoming requests, enforces JWT authentication, validates tool call arguments against runtime schemas, and forwards execution to configured actions.

## Features

- Accepts JSON-RPC 2.0 requests over HTTP POST at `/mcp/{server-id}`
- JWT-based authentication (expects `Authorization: Bearer <token>`)
- Runtime validation of tool arguments against tool input schemas
- Executes mapped actions and returns standardized `CallToolResult`
- Defensive limits (request body limited to **10 MB**) and content-type validation

## HTTP contract

- Endpoint: `POST /mcp/{server-id}`
- Required headers:
	- `Content-Type: application/json`
	- `Authorization: Bearer <token>`
- Body: JSON-RPC 2.0 request object. Supported methods:
	- `initialize` — negotiate protocol version and capabilities
	- `tools/list` — returns the list of tools available on the server
	- `tools/call` — call a tool by id with `arguments` (validated against the tool's input schema)

Example `tools/call` request body:

```json
{
	"jsonrpc": "2.0",
	"id": 1,
	"method": "tools/call",
	"params": {
		"tool_id": "weather:get",
		"arguments": { "location": "Amsterdam" }
	}
}
```

Example response (success):

```json
{
	"jsonrpc": "2.0",
	"id": 1,
	"result": { /* CallToolResult */ }
}
```

Errors follow JSON-RPC conventions. Common codes returned by this component include:

- `-32700` — Parse error (invalid JSON)
- `-32600` — Invalid Request
- `-32601` — Method not found
- `-32603` — Internal error
- `-32000` — Component-level/custom errors (e.g., Unauthorized)

## Configuration

Configuration is delivered via the wasi:config store and must contain a `mcp-servers` array. Each entry looks like:

```json
{
	"mcp-servers": [
		{
			"id": "weather",
			"tools": [
				{
					"id": "weather:get",
					"name": "Get weather",
					"description": "Retrieve weather by location",
					"action-id": "weather.get",
					"input": {
						"type": "object",
						"properties": {
							"location": { "type": "string" }
						},
						"required": ["location"]
					}
				}
			]
		}
	]
}
```

The component deserializes the config into `McpServersConfig` (`mcp-servers`) and expects each tool to include an `action-id` that maps to an action this component can invoke.

## Security & limits

- Requests are limited to **10 MB** to guard against malicious payloads and DoS attempts.
- Requests must have `Content-Type: application/json`.
- JWT verification is enforced — requests without a valid token will return `Unauthorized`.


## Build & test

From the component directory:

- Run unit tests: `cargo test`
- Build the WASM: `wash build` (requires `wash` and the wasm toolchain)
- Format & lint: `cargo fmt` and `cargo clippy` (or `just lint-all` from repo root)
