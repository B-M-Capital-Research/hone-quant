#!/usr/bin/env bash
# Installs a release tarball and switches to it atomically, rolling back if it does not come up.
#
#   sudo scripts/deploy.sh dist/hone-quant-<version>-<rev>-linux-x86_64.tar.gz
#
# Steps: verify checksums → unpack into /opt/hone-quant/releases/<name> → run migrations as the
# service user → switch /opt/hone-quant/current → restart → wait for /api/health to report the
# new revision → otherwise switch back and restart the previous release. Keeps three releases.
set -euo pipefail

[[ $EUID -eq 0 ]] || { echo "error: run as root" >&2; exit 1; }
tarball="${1:?usage: deploy.sh <release.tar.gz>}"
base=/opt/hone-quant
releases="$base/releases"
env_file=/etc/hone-quant/runtime.env
keep=3

[[ -f "$env_file" ]] || { echo "error: $env_file is missing (run scripts/install-host.sh first)" >&2; exit 1; }
if [[ -f "$tarball.sha256" ]]; then
  (cd "$(dirname "$tarball")" && sha256sum --check --status "$(basename "$tarball").sha256") \
    || { echo "error: checksum mismatch for $tarball" >&2; exit 1; }
fi

# sed reads the whole listing: `| head -n 1` would SIGPIPE tar, which pipefail turns into an exit.
name="$(tar -tzf "$tarball" | sed -n '1s,/.*,,p')"
[[ "$name" =~ ^hone-quant-[A-Za-z0-9._-]+$ ]] || { echo "error: unexpected archive layout" >&2; exit 1; }
target="$releases/$name"
if [[ -e "$target" ]]; then
  echo "release $name is already unpacked; reusing it"
else
  tmp="$(mktemp -d "$releases/.unpack.XXXXXX")"
  tar -xzf "$tarball" -C "$tmp" --no-same-owner
  (cd "$tmp/$name" && sha256sum --check --status SHA256SUMS) || { rm -rf -- "$tmp"; echo "error: SHA256SUMS mismatch" >&2; exit 1; }
  mv "$tmp/$name" "$target"
  rmdir "$tmp"
fi
revision="$(sed -n 's/^revision=//p' "$target/RELEASE")"
echo "release: $name ($revision)"

run_as_service() {
  # The same environment the service gets, without starting it.
  systemd-run --quiet --wait --pipe --collect --uid=hone-quant --gid=hone-quant \
    --property=EnvironmentFile="$env_file" \
    --setenv=HONE_QUANT_STATE_DIR=/var/lib/hone-quant \
    --working-directory=/var/lib/hone-quant "$@"
}

echo "==> migrations"
run_as_service "$target/hone-quant" migrate

previous="$(readlink -f "$base/current" 2>/dev/null || true)"
switch_to() {
  ln -sfn "$1" "$base/current.next"
  mv -T "$base/current.next" "$base/current"
}

bind="$(sed -n 's/^HONE_QUANT_BIND=//p' "$env_file" | tail -n 1)"
bind="${bind:-127.0.0.1:8090}"
health_url="http://${bind/0.0.0.0/127.0.0.1}/api/health"

wait_healthy() {
  local want="$1"
  for _ in $(seq 1 60); do
    if body="$(curl -fsS --max-time 3 "$health_url" 2>/dev/null)" && [[ "$body" == *"\"ok\":true"* && "$body" == *"\"revision\":\"$want\""* ]]; then
      return 0
    fi
    sleep 1
  done
  return 1
}

echo "==> switch and restart"
switch_to "$target"
systemctl restart hone-quant
if wait_healthy "$revision"; then
  echo "hone-quant $revision is healthy at $health_url"
else
  echo "error: the new release did not become healthy; recent logs:" >&2
  journalctl -u hone-quant -n 40 --no-pager >&2 || true
  if [[ -n "$previous" && -d "$previous" ]]; then
    echo "rolling back to $(basename "$previous")" >&2
    switch_to "$previous"
    systemctl restart hone-quant
  fi
  exit 1
fi

# install-host.sh enables the backup timer, but an enabled timer only starts at the next boot.
# Start it now (a no-op when it already runs); a timer an operator disabled stays off.
if systemctl is-enabled --quiet hone-quant-backup.timer 2>/dev/null; then
  systemctl start hone-quant-backup.timer
fi

# Retention: the current release plus the newest others, up to $keep in total.
current="$(readlink -f "$base/current")"
mapfile -t stale < <(find "$releases" -mindepth 1 -maxdepth 1 -type d -name 'hone-quant-*' -printf '%T@ %p\n' \
  | sort -rn | cut -d' ' -f2- | grep -vxF "$current" | tail -n +"$keep")
for dir in "${stale[@]}"; do
  [[ "$dir" == "$releases"/hone-quant-* && ! -L "$dir" ]] && rm -rf -- "$dir"
done
