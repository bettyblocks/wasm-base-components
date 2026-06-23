#!/bin/bash
# MCP Initialize - negotiate protocol version and capabilities

SERVER_ID="${1:-test}"
BASE_URL="http://localhost:8888"

curl -s -X POST "${BASE_URL}/api/mcp/${SERVER_ID}" \
  -H "Host: testt-test" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "initialize",
    "params": {
      "protocolVersion": "2024-11-05",
      "capabilities": {},
      "clientInfo": {
        "name": "curl-test",
        "version": "1.0.0"
      }
    }
  }' | jq . 2>/dev/null || cat
