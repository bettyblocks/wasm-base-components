#!/bin/bash
# Error test - wrong HTTP method (expect 405)

SERVER_ID="${1:-test}"
BASE_URL="http://localhost:8888"

curl -s -w "\nHTTP Status: %{http_code}\n" -X GET "${BASE_URL}/api/mcp/${SERVER_ID}" \
  -H "Host: testt-test"
