#!/usr/bin/env bash
# Admin API cookbook — point at a running engine.
#   BASE=http://127.0.0.1:9678  HARVEST_ADMIN_TOKEN=…(if the node has one)
set -euo pipefail
BASE="${BASE:-http://127.0.0.1:9678}"
TOKEN="${HARVEST_ADMIN_TOKEN:-}"

echo "== liveness (always auth-exempt; safe for probes) =="
curl -fsS "$BASE/health"; echo

echo "== unauthenticated call must get 401 (no token) or work (no auth configured) =="
curl -s -o /dev/null -w '%{http_code}\n' "$BASE/stats" || true

auth=()
if [[ -n "$TOKEN" ]]; then auth=(-H "Authorization: Bearer $TOKEN"); fi

echo "== live snapshot (counters + proxy health; URLs never exposed) =="
curl -fsS "${auth[@]}" "$BASE/stats" | head -40

echo "== prometheus scrape text =="
curl -fsS "${auth[@]}" "$BASE/metrics" | head -20

echo "== recent runs (history tail) =="
curl -fsS "${auth[@]}" "$BASE/recent"

echo "== policy self-description =="
curl -fsS "${auth[@]}" "$BASE/policy"

echo "== security headers present on every response =="
# (GET + dump headers; the server routes by method and path. Mutations
#  are limited to explicit POST endpoints documented in docs/api/.)
curl -fsS -D - -o /dev/null "${auth[@]}" "$BASE/stats" \
  | grep -iE 'x-content-type-options|cache-control'

echo "== unknown route: 404 =="
curl -s -o /dev/null -w '%{http_code}\n' "$BASE/nope" || true

# Negative tests worth running before a security review:
#   wrong token        -> 401, body carries no echo of path/token
#   token set, /health -> 200 (probes must never authenticate)
#   POST /stats        -> 405 (GET-only route)
#   unsupported method -> 405; the six documented POST routes are explicit
#                         operator actions or bounded control-plane operations
