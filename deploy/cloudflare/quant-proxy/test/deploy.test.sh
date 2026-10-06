#!/usr/bin/env bash
# End-to-end test of deploy.sh against mock_cloudflare.py (no network, no real account).
#   deploy/cloudflare/quant-proxy/test/deploy.test.sh
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
deploy="$here/../deploy.sh"
port="${MOCK_PORT:-18765}"
api_token="test-api-token-$(date +%s)"
origin_token="$(printf 'o%.0s' {1..64})"
log="$(mktemp -d)"
trap 'kill "$mock_pid" 2>/dev/null || true; rm -rf -- "$log"' EXIT

MOCK_API_TOKEN="$api_token" MOCK_EXPECTED_SECRET="$origin_token" python3 "$here/mock_cloudflare.py" "$port" &
mock_pid=$!
for _ in $(seq 1 50); do curl -s "http://127.0.0.1:$port/_state" >/dev/null && break; sleep 0.1; done

export CLOUDFLARE_API_BASE="http://127.0.0.1:$port/client/v4"
export CLOUDFLARE_API_TOKEN="$api_token"
export CLOUDFLARE_ACCOUNT_ID=0123456789abcdef0123456789abcdef
export CLOUDFLARE_ZONE_ID=fedcba9876543210fedcba9876543210
export QUANT_ORIGIN_URL=https://origin.example.com
export HONE_QUANT_PUBLIC_URL="http://127.0.0.1:$port/quant"
failures=0
state() { curl -s "http://127.0.0.1:$port/_state" | python3 -c "import json, sys; s = json.load(sys.stdin); print($1)"; }
expect() { # description, expected, actual
  if [[ "$2" == "$3" ]]; then echo "ok   - $1"; else echo "FAIL - $1: expected [$2], got [$3]"; failures=$((failures + 1)); fi
}
run() { # name, then the command; records exit status and output
  local name="$1"; shift
  set +e; "$@" >"$log/$name.out" 2>&1; echo $? >"$log/$name.rc"; set -e
}

run dry "$deploy" --dry-run </dev/null
expect "dry run succeeds" 0 "$(cat "$log/dry.rc")"
expect "dry run changes nothing" "0 0" "$(state 'len(s["routes"]), len(s["scripts"])')"

run nosecret "$deploy" --no-verify </dev/null
expect "first deploy without the origin token fails" 1 "$(cat "$log/nosecret.rc")"
expect "...and creates no route" 0 "$(state 'len(s["routes"])')"

run first bash -c "printf '%s\n' '$origin_token' | '$deploy'"
expect "deploy with the token on stdin succeeds" 0 "$(cat "$log/first.rc")"
expect "one route, bound to the worker" "hone-claw.com/quant* hone-quant-proxy" \
  "$(state '" ".join([s["routes"][0]["pattern"], s["routes"][0]["script"]]) if len(s["routes"]) == 1 else s["routes"]')"
expect "module uploaded as an ES module" "application/javascript+module index.js" \
  "$(state '" ".join([s["scripts"]["hone-quant-proxy"]["module_type"], s["scripts"]["hone-quant-proxy"]["metadata"]["main_module"]])')"
expect "origin URL var and kept secrets" "[{'type': 'plain_text', 'name': 'QUANT_ORIGIN_URL', 'text': 'https://origin.example.com'}] ['secret_text']" \
  "$(state 'str(s["scripts"]["hone-quant-proxy"]["metadata"]["bindings"]) + " " + str(s["scripts"]["hone-quant-proxy"]["metadata"]["keep_bindings"])')"
expect "secret set with the piped value" "True" "$(state 's["scripts"]["hone-quant-proxy"]["secrets"]["QUANT_ORIGIN_TOKEN"]["matched_expected"]')"
expect "workers.dev and previews off" "{'enabled': False, 'previews_enabled': False}" "$(state 's["scripts"]["hone-quant-proxy"]["subdomain"]')"
expect "verification passed" 1 "$(grep -c 'hone-quant is live' "$log/first.out")"

run again "$deploy" </dev/null
expect "redeploy without stdin keeps the secret" 0 "$(cat "$log/again.rc")"
expect "...and does not duplicate the route" 1 "$(state 'len(s["routes"])')"
expect "...secret still present" "True" "$(state 's["scripts"]["hone-quant-proxy"]["secrets"]["QUANT_ORIGIN_TOKEN"]["matched_expected"]')"

run rollback "$deploy" --rollback
expect "rollback succeeds" 0 "$(cat "$log/rollback.rc")"
expect "...and removes the route" 0 "$(state 'len(s["routes"])')"
run rollback2 "$deploy" --rollback
expect "a second rollback is a no-op" "0 1" "$(cat "$log/rollback2.rc") $(grep -c 'nothing to delete' "$log/rollback2.out")"

curl -s -X POST "http://127.0.0.1:$port/_route" --data '{"pattern":"hone-claw.com/quant*","script":"someone-else"}' >/dev/null
run conflict "$deploy" --no-verify </dev/null
expect "a route owned by another worker blocks the deploy" "1 1" \
  "$(cat "$log/conflict.rc") $(grep -c 'bound to someone-else' "$log/conflict.out")"
run conflictdry "$deploy" --dry-run </dev/null
expect "...and the dry run reports it" 1 "$(cat "$log/conflictdry.rc")"

run delete "$deploy" --rollback --delete-worker --no-verify
expect "rollback --delete-worker removes the script" "0 0" "$(cat "$log/delete.rc") $(state 'len(s["scripts"])')"

run badtoken env CLOUDFLARE_API_TOKEN=wrong "$deploy" --dry-run
expect "a bad API token is reported" "1 1" "$(cat "$log/badtoken.rc") $(grep -c 'Invalid access token' "$log/badtoken.out")"

leaks="$(cat "$log"/*.out | grep -c -e "$origin_token" -e "$api_token" || true)"
expect "no secret appears in any output" 0 "$leaks"

if [[ $failures -gt 0 ]]; then
  echo "$failures check(s) failed; outputs:"
  for f in "$log"/*.out; do echo "--- $(basename "$f")"; cat "$f"; done
  exit 1
fi
echo "all deploy.sh checks passed"
