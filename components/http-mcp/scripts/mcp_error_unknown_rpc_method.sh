#!/bin/bash
# Error test - unknown RPC method (expect method-not-found error)

SERVER_ID="${1:-test}"
BASE_URL="http://localhost:8888"

curl -s -X POST "${BASE_URL}/api/mcp/${SERVER_ID}" \
  -H "Host: testt-test" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 5,
    "method": "nonexistent/method",
    "params": {}
  }' | jq . 2>/dev/null || cat
