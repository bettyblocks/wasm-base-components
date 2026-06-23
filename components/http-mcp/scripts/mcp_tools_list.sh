#!/bin/bash
# MCP Tools List - get list of available tools

SERVER_ID="${1:-test}"
BASE_URL="http://localhost:8888"

curl -s -X POST "${BASE_URL}/api/mcp/${SERVER_ID}" \
  -H "Host: testt-test" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 2,
    "method": "tools/list",
    "params": {}
  }' | jq . 2>/dev/null || cat
