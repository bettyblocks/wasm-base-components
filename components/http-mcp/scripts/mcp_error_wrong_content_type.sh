#!/bin/bash
# Error test - wrong Content-Type (expect 400)

SERVER_ID="${1:-test}"
BASE_URL="http://localhost:8888"

curl -s -w "\nHTTP Status: %{http_code}\n" -X POST "${BASE_URL}/api/mcp/${SERVER_ID}" \
  -H "Host: testt-test" \
  -H "Content-Type: text/plain" \
  -d '{"jsonrpc":"2.0","id":4,"method":"initialize","params":{}}' | jq . 2>/dev/null || cat
