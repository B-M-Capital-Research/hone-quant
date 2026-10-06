#!/usr/bin/env bash
# Deploys the hone-quant-proxy Worker and its hone-claw.com/quant* route with the Cloudflare API
# (no wrangler needed), or takes them out again. Needs bash, curl and python3.
#
#   export CLOUDFLARE_API_TOKEN=...   # Account: Workers Scripts Edit; Zone hone-claw.com: Workers Routes Edit
#   export CLOUDFLARE_ACCOUNT_ID=... CLOUDFLARE_ZONE_ID=...   # the account and the hone-claw.com zone
#   export QUANT_ORIGIN_URL=https://<origin-host>             # not needed for --rollback
#   ./deploy.sh --dry-run             # check the token and the zone's /quant routes; change nothing
#   ssh <origin> "sudo sed -n 's/^HONE_QUANT_ORIGIN_TOKEN=//p' /etc/hone-quant/runtime.env" | ./deploy.sh
#   ./deploy.sh                       # code only: keeps the QUANT_ORIGIN_TOKEN secret already set
#   ./deploy.sh --rollback            # delete the route: /quant is served by the Pages site again
#   ./deploy.sh --rollback --delete-worker
#
# The origin token is read from stdin, never from argv; the API token goes to curl through a
# private config file. Nothing secret is printed. Order on deploy: upload the script (keeping
# existing secrets), set the secret, turn off *.workers.dev and preview URLs, and only then create
# the route, so no request can reach a half-configured Worker. Afterwards the public URL is
# checked: health 200, /api/auth/me 401 without a session, /quant 308 to /quant/.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
api="${CLOUDFLARE_API_BASE:-https://api.cloudflare.com/client/v4}"
account="${CLOUDFLARE_ACCOUNT_ID:-}"
zone="${CLOUDFLARE_ZONE_ID:-}"
script="${QUANT_WORKER_NAME:-hone-quant-proxy}"
pattern="${QUANT_ROUTE_PATTERN:-hone-claw.com/quant*}"
origin_url="${QUANT_ORIGIN_URL:-}"
public_url="${HONE_QUANT_PUBLIC_URL:-https://hone-claw.com/quant}"
compatibility_date=2026-09-15

mode=deploy
delete_worker=false
verify=true
for arg in "$@"; do
  case "$arg" in
    --dry-run) mode=dry-run ;;
    --rollback) mode=rollback ;;
    --delete-worker) delete_worker=true ;;
    --no-verify) verify=false ;;
    *) echo "usage: $0 [--dry-run | --rollback [--delete-worker]] [--no-verify]" >&2; exit 2 ;;
  esac
done
[[ $delete_worker == false || $mode == rollback ]] || { echo "error: --delete-worker goes with --rollback" >&2; exit 2; }
[[ -n "${CLOUDFLARE_API_TOKEN:-}" ]] || { echo "error: set CLOUDFLARE_API_TOKEN" >&2; exit 2; }
[[ -n "$account" && -n "$zone" ]] || { echo "error: set CLOUDFLARE_ACCOUNT_ID and CLOUDFLARE_ZONE_ID" >&2; exit 2; }
[[ $mode == rollback || -n "$origin_url" ]] || { echo "error: set QUANT_ORIGIN_URL (https://<origin-host>)" >&2; exit 2; }
command -v python3 >/dev/null || { echo "error: python3 is required" >&2; exit 2; }

umask 077
tmp="$(mktemp -d)"
trap 'rm -rf -- "$tmp"' EXIT
printf 'header = "Authorization: Bearer %s"\n' "$CLOUDFLARE_API_TOKEN" >"$tmp/auth"

# py SCRIPT_ON_STDIN [ARGS...]: small JSON helpers; data goes through files and the environment.
py() { python3 - "$@"; }
api_ok() {
  python3 -c 'import json, sys; sys.exit(0 if json.load(open(sys.argv[1])).get("success") else 1)' "$1" 2>/dev/null
}
api_errors() {
  python3 -c 'import json, sys
d = json.load(open(sys.argv[1]))
errors = d.get("errors") or []
print("; ".join(str(e.get("code")) + ": " + str(e.get("message")) for e in errors) or "unsuccessful")' "$1" 2>/dev/null \
    || echo "no JSON body"
}

# cf METHOD PATH [curl args...]: prints the JSON response; fails unless the API says success.
cf() {
  local method="$1" path="$2" code
  shift 2
  if ! code="$(curl -sS -K "$tmp/auth" -X "$method" -o "$tmp/out.json" -w '%{http_code}' "$@" "$api$path")"; then
    echo "error: $method $path: request failed (network)" >&2
    return 1
  fi
  if ! api_ok "$tmp/out.json"; then
    echo "error: $method $path -> HTTP $code: $(api_errors "$tmp/out.json")" >&2
    return 1
  fi
  cat "$tmp/out.json"
}

echo "==> API token"
if cf GET /user/tokens/verify >"$tmp/verify.json" 2>"$tmp/verify.err" \
  || cf GET "/accounts/$account/tokens/verify" >"$tmp/verify.json" 2>"$tmp/verify.err"; then
  py "$tmp/verify.json" <<'PY'
import json, sys
print("    status:", json.load(open(sys.argv[1]))["result"].get("status"))
PY
else
  cat "$tmp/verify.err" >&2
  exit 1
fi

echo "==> routes on the zone matching /quant"
cf GET "/zones/$zone/workers/routes" >"$tmp/routes.json"
# Prints "id<TAB>pattern<TAB>script" for every route whose pattern covers /quant.
py "$tmp/routes.json" <<'PY' >"$tmp/quant-routes.tsv"
import json, sys
for r in json.load(open(sys.argv[1]))["result"] or []:
    p = r.get("pattern", "")
    if "/quant" in p:
        print(f"{r.get('id')}\t{p}\t{r.get('script') or '(none)'}")
PY
if [[ -s "$tmp/quant-routes.tsv" ]]; then sed 's/^/    /' "$tmp/quant-routes.tsv"; else echo "    (none)"; fi
ours="$(awk -F'\t' -v p="$pattern" -v s="$script" '$2 == p && $3 == s { print $1 }' "$tmp/quant-routes.tsv")"
foreign="$(awk -F'\t' -v p="$pattern" -v s="$script" '$2 == p && $3 != s { print $3 }' "$tmp/quant-routes.tsv")"

if [[ $mode == dry-run ]]; then
  if [[ -n "$foreign" ]]; then echo "conflict: $pattern is bound to $foreign"; exit 1; fi
  cf GET "/accounts/$account/workers/scripts/$script/secrets" >"$tmp/secrets.json" 2>/dev/null \
    && echo "==> worker $script exists" || echo "==> worker $script does not exist yet"
  echo "dry run: would upload $script (QUANT_ORIGIN_URL=$origin_url), set QUANT_ORIGIN_TOKEN when piped in,"
  echo "         disable workers.dev and preview URLs, and $([[ -n "$ours" ]] && echo keep || echo create) route $pattern"
  exit 0
fi

if [[ $mode == rollback ]]; then
  if [[ -z "$ours" ]]; then
    echo "==> no route $pattern for $script; nothing to delete"
  else
    for id in $ours; do
      cf DELETE "/zones/$zone/workers/routes/$id" >/dev/null
      echo "==> deleted route $pattern ($id)"
    done
  fi
  if [[ $delete_worker == true ]]; then
    cf DELETE "/accounts/$account/workers/scripts/$script?force=true" >/dev/null
    echo "==> deleted worker $script"
  fi
  if [[ $verify == true ]]; then
    code="$(curl -sS -o /dev/null -w '%{http_code}' --max-time 15 "$public_url/api/health" || true)"
    echo "==> $public_url/api/health now answers $code (served by the zone again, not hone-quant)"
  fi
  exit 0
fi

[[ -z "$foreign" ]] || { echo "error: route $pattern is bound to $foreign; resolve that first" >&2; exit 1; }

origin_token=""
if [[ ! -t 0 ]]; then
  IFS= read -r origin_token || true
  origin_token="$(printf '%s' "$origin_token" | tr -d '\r' | sed 's/^[[:space:]]*//; s/[[:space:]]*$//')"
  if [[ -n "$origin_token" ]] && { [[ ${#origin_token} -lt 32 ]] || [[ ! "$origin_token" =~ ^[[:graph:]]+$ ]]; }; then
    echo "error: the origin token on stdin must be at least 32 printable characters" >&2
    exit 1
  fi
fi

echo "==> upload $script"
QUANT_ORIGIN_URL="$origin_url" COMPAT="$compatibility_date" py "$tmp/metadata.json" <<'PY'
import json, os, sys
json.dump({
    "main_module": "index.js",
    "compatibility_date": os.environ["COMPAT"],
    "bindings": [{"type": "plain_text", "name": "QUANT_ORIGIN_URL", "text": os.environ["QUANT_ORIGIN_URL"]}],
    # Redeploys keep the QUANT_ORIGIN_TOKEN secret set earlier.
    "keep_bindings": ["secret_text"],
}, open(sys.argv[1], "w"))
PY
cf PUT "/accounts/$account/workers/scripts/$script" \
  -F "metadata=@$tmp/metadata.json;type=application/json" \
  -F "index.js=@$here/src/index.js;type=application/javascript+module" >/dev/null

if [[ -n "$origin_token" ]]; then
  echo "==> set secret QUANT_ORIGIN_TOKEN"
  TOKEN="$origin_token" py "$tmp/secret.json" <<'PY'
import json, os, sys
json.dump({"name": "QUANT_ORIGIN_TOKEN", "text": os.environ["TOKEN"], "type": "secret_text"}, open(sys.argv[1], "w"))
PY
  cf PUT "/accounts/$account/workers/scripts/$script/secrets" \
    -H 'Content-Type: application/json' --data "@$tmp/secret.json" >/dev/null
  rm -f -- "$tmp/secret.json"
  origin_token=""
fi
cf GET "/accounts/$account/workers/scripts/$script/secrets" >"$tmp/secrets.json"
if ! py "$tmp/secrets.json" <<'PY'; then
import json, sys
names = {s.get("name") for s in json.load(open(sys.argv[1]))["result"] or []}
sys.exit(0 if "QUANT_ORIGIN_TOKEN" in names else 1)
PY
  echo "error: $script has no QUANT_ORIGIN_TOKEN secret; pipe the origin token in (see the header)." >&2
  echo "       The script is uploaded but no route points at it." >&2
  exit 1
fi

echo "==> workers.dev and preview URLs off"
cf POST "/accounts/$account/workers/scripts/$script/subdomain" \
  -H 'Content-Type: application/json' --data '{"enabled":false,"previews_enabled":false}' >/dev/null

if [[ -n "$ours" ]]; then
  echo "==> route $pattern already points at $script"
else
  echo "==> create route $pattern"
  PATTERN="$pattern" SCRIPT="$script" py "$tmp/route.json" <<'PY'
import json, os, sys
json.dump({"pattern": os.environ["PATTERN"], "script": os.environ["SCRIPT"]}, open(sys.argv[1], "w"))
PY
  cf POST "/zones/$zone/workers/routes" -H 'Content-Type: application/json' --data "@$tmp/route.json" >/dev/null
fi

[[ $verify == true ]] || exit 0
echo "==> verify $public_url"
check() { # label, expected status, url
  local got
  for _ in $(seq 1 12); do
    got="$(curl -sS -o "$tmp/body" -w '%{http_code}' --max-time 15 "$3" || true)"
    [[ "$got" == "$2" ]] && break
    sleep 5
  done
  printf '    %-34s %s (want %s)\n' "$1" "$got" "$2"
  [[ "$got" == "$2" ]]
}
ok=true
check "health" 200 "$public_url/api/health" && grep -Eq '"ok"[[:space:]]*:[[:space:]]*true' "$tmp/body" || ok=false
check "auth/me without a session" 401 "$public_url/api/auth/me" || ok=false
check "bare path redirects" 308 "$public_url" || ok=false
[[ $ok == true ]] && echo "hone-quant is live at $public_url/" || { echo "error: verification failed" >&2; exit 1; }
