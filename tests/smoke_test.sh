#!/usr/bin/env bash
# Smoke test for the Vigil REST API.
# Run with: bash tests/smoke_test.sh
# Requires: vigil running (cargo run -- -b 127.0.0.1:3030) and jq installed.
set -euo pipefail

BASE="http://127.0.0.1:3030/api/v1"

echo "=== Health check ==="
curl -sf "$BASE/health" | jq .

echo ""
echo "=== Engine stats ==="
curl -sf "$BASE/stats" | jq .

echo ""
echo "=== List nodes (should be empty) ==="
curl -sf "$BASE/nodes" | jq .

echo ""
echo "=== Add a node ==="
NODE=$(curl -sf -X POST "$BASE/nodes" \
  -H 'Content-Type: application/json' \
  -d '{"name":"test-router","addresses":["127.0.0.1"],"polling_profile":"default"}')
echo "$NODE" | jq .
NODE_ID=$(echo "$NODE" | jq -r '.id')

echo ""
echo "=== Get node by ID ==="
curl -sf "$BASE/nodes/$NODE_ID" | jq .

echo ""
echo "=== Get node status ==="
curl -sf "$BASE/nodes/$NODE_ID/status" | jq .

echo ""
echo "=== Add a TCP poll (port 22, 30s interval) ==="
curl -sf -X POST "$BASE/nodes/$NODE_ID/polls" \
  -H 'Content-Type: application/json' \
  -d '{"protocol":{"TcpConnect":{"port":22}},"interval_secs":30,"timeout_ms":5000}' | jq .

echo ""
echo "=== List nodes (should show our node) ==="
curl -sf "$BASE/nodes" | jq .

echo ""
echo "=== Waiting 35s for a poll cycle... ==="
sleep 35

echo ""
echo "=== Check status after poll ==="
curl -sf "$BASE/nodes/$NODE_ID/status" | jq .

echo ""
echo "=== Get poll results ==="
curl -sf "$BASE/nodes/$NODE_ID/results" | jq .

echo ""
echo "=== Engine stats ==="
curl -sf "$BASE/stats" | jq .

echo ""
echo "=== Delete node ==="
curl -sf -X DELETE "$BASE/nodes/$NODE_ID" | jq .

echo ""
echo "=== List nodes (should be empty again) ==="
curl -sf "$BASE/nodes" | jq .

echo ""
echo "=== SMOKE TEST PASSED ==="
