#!/bin/bash
# MCP Tools Call - execute a tool with provided arguments

SERVER_ID="${1:-test}"
TOOL_NAME="${2:-your-tool-name}"
ARGUMENTS="${3:-'{}'}"
BASE_URL="http://localhost:8888"

curl -s -X POST "${BASE_URL}/api/mcp/${SERVER_ID}" \
  -H "Host: testt-test" \
  -H "Content-Type: application/json" \
  -d "{
    \"jsonrpc\": \"2.0\",
    \"id\": 3,
    \"method\": \"tools/call\",
    \"params\": {
      \"name\": \"${TOOL_NAME}\",
      \"arguments\": ${ARGUMENTS}
    }
  }" | jq . 2>/dev/null || cat
