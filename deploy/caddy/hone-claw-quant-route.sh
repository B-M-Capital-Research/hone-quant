#!/usr/bin/env bash
# Routes hone-claw.com/quant on honeclaw's origin Caddy to hone-quant (run as root on that host).
#
#   sudo deploy/caddy/hone-claw-quant-route.sh            add the route (idempotent)
#   sudo deploy/caddy/hone-claw-quant-route.sh --remove   take it out again (rollback)
#   ... --before-line '# BEGIN SOME SECTION'            put the block before that line instead
#
# Adds one marked block to the site that serves {$HONE_ORIGIN_HOST}: /quant and /quant/* go to
# hone-quant on 127.0.0.1:8090 and every other path keeps flowing to honeclaw exactly as before.
# The block goes first in that site, or right before --before-line when another tool rewrites a
# section of the site; re-run this script if the Caddyfile is ever restored from an older copy.
# Caddy needs no secret for this route: hone-quant answers 404 to anything that lacks the
# Cloudflare Worker's X-Hone-Quant-Origin-Token (HONE_QUANT_ORIGIN_TOKEN).
#
# The edited file is validated with the caddy unit's environment before it replaces the current
# one, the current one is kept as Caddyfile.bak-hone-quant-<UTC time>, and a failed reload puts
# it back.
set -euo pipefail

caddyfile=/etc/caddy/Caddyfile
env_file=/etc/hone/origin.env
upstream=127.0.0.1:8090
before_line=
action=add
while [[ $# -gt 0 ]]; do
  case "$1" in
    --remove) action=remove ;;
    --caddyfile) caddyfile="${2:?}"; shift ;;
    --env) env_file="${2:?}"; shift ;;
    --upstream) upstream="${2:?}"; shift ;;
    --before-line) before_line="${2:?}"; shift ;;
    *) echo "usage: $0 [--remove] [--caddyfile FILE] [--env FILE] [--upstream HOST:PORT] [--before-line LINE]" >&2; exit 2 ;;
  esac
  shift
done
[[ $EUID -eq 0 ]] || { echo "error: run as root" >&2; exit 1; }
[[ -f "$caddyfile" ]] || { echo "error: $caddyfile not found" >&2; exit 1; }
[[ "$upstream" =~ ^[A-Za-z0-9.:-]+$ ]] || { echo "error: bad --upstream (expected HOST:PORT)" >&2; exit 2; }

begin='# BEGIN HONE-QUANT'
end='# END HONE-QUANT'
if grep -qF "$begin" "$caddyfile"; then present=true; else present=false; fi
if [[ $action == add && $present == true ]]; then echo "the /quant route is already in $caddyfile"; exit 0; fi
if [[ $action == remove && $present == false ]]; then echo "no /quant route in $caddyfile; nothing to do"; exit 0; fi

# Secrets from the environment file must never reach the terminal.
mask() { sed -E 's/[A-Za-z0-9_+\/=-]{24,}/<masked>/g'; }

tmp="$(mktemp "$(dirname "$caddyfile")/.Caddyfile.hone-quant.XXXXXX")"
log="$(mktemp)"
trap 'rm -f -- "$tmp" "$log"' EXIT

# Lines are compared trimmed and as fixed strings (no regex surprises with "{$...}").
count_lines() {
  ANCHOR="$1" awk '{ l = $0; sub(/^[ \t]+/, "", l); sub(/[ \t]+$/, "", l) }
    l == ENVIRON["ANCHOR"] { n++ } END { print n + 0 }' "$caddyfile"
}

if [[ $action == add ]]; then
  # Right before --before-line when given, otherwise first thing in the origin site.
  if [[ -n "$before_line" ]]; then
    anchor="$before_line"
    where=before
  else
    anchor='{$HONE_ORIGIN_HOST} {'
    where=after
  fi
  found="$(count_lines "$anchor")"
  [[ "$found" == 1 ]] || { echo "error: expected one line '$anchor' in $caddyfile, found $found" >&2; exit 1; }
  ANCHOR="$anchor" WHERE="$where" UPSTREAM="$upstream" awk '
    function block() {
      print "\t# BEGIN HONE-QUANT: hone-claw.com/quant -> hone-quant (deploy/caddy/hone-claw-quant-route.sh)"
      print "\t@hone_quant path /quant /quant/*"
      print "\thandle @hone_quant {"
      print "\t\treverse_proxy " ENVIRON["UPSTREAM"]
      print "\t}"
      print "\t# END HONE-QUANT"
    }
    { l = $0; sub(/^[ \t]+/, "", l); sub(/[ \t]+$/, "", l) }
    l == ENVIRON["ANCHOR"] && ENVIRON["WHERE"] == "before" { block() }
    { print }
    l == ENVIRON["ANCHOR"] && ENVIRON["WHERE"] == "after" { block() }
  ' "$caddyfile" >"$tmp"
else
  BEGIN_MARK="$begin" END_MARK="$end" awk '
    index($0, ENVIRON["BEGIN_MARK"]) { skip = 1 }
    !skip { print }
    skip && index($0, ENVIRON["END_MARK"]) { skip = 0 }
  ' "$caddyfile" >"$tmp"
fi

# Validate with the same environment the caddy unit gets ({$HONE_ORIGIN_HOST} and friends).
if ! (
  set -a
  # shellcheck disable=SC1090
  [[ -f "$env_file" ]] && . "$env_file"
  set +a
  caddy validate --config "$tmp" --adapter caddyfile
) >"$log" 2>&1; then
  echo "error: the edited Caddyfile does not validate; nothing changed:" >&2
  tail -n 20 "$log" | mask >&2
  exit 1
fi

backup="$caddyfile.bak-hone-quant-$(date -u +%Y%m%dT%H%M%SZ)"
cp -p -- "$caddyfile" "$backup"
chown --reference="$caddyfile" -- "$tmp"
chmod --reference="$caddyfile" -- "$tmp"
mv -f -- "$tmp" "$caddyfile"
if ! systemctl reload caddy; then
  echo "error: caddy did not accept the new configuration; restoring $backup" >&2
  cp -p -- "$backup" "$caddyfile"
  systemctl reload caddy || true
  exit 1
fi
diff -u "$backup" "$caddyfile" | mask || true
echo "caddy reloaded (${action}); previous Caddyfile kept as $backup"
