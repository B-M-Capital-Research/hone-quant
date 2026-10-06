#!/usr/bin/env bash
# One-time preparation of a Linux host (run as root): service user, directories, systemd units
# and the runtime.env template. Safe to re-run; never overwrites an existing runtime.env.
#
#   sudo scripts/install-host.sh [path/to/deploy]     (defaults to ../deploy next to this script)
set -euo pipefail

[[ $EUID -eq 0 ]] || { echo "error: run as root" >&2; exit 1; }
deploy_dir="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../deploy" && pwd)}"
[[ -f "$deploy_dir/systemd/hone-quant.service" ]] || { echo "error: $deploy_dir has no systemd/hone-quant.service" >&2; exit 1; }

if ! id hone-quant >/dev/null 2>&1; then
  useradd --system --home-dir /var/lib/hone-quant --shell /usr/sbin/nologin --user-group hone-quant
  echo "created user hone-quant"
fi

install -d -m 0755 -o root -g root /opt/hone-quant /opt/hone-quant/releases
install -d -m 0750 -o root -g hone-quant /etc/hone-quant
install -d -m 0700 -o hone-quant -g hone-quant /var/lib/hone-quant /var/backups/hone-quant

if [[ ! -f /etc/hone-quant/runtime.env ]]; then
  install -m 0640 -o root -g hone-quant "$deploy_dir/runtime.env.example" /etc/hone-quant/runtime.env
  echo "wrote /etc/hone-quant/runtime.env — edit it before the first start"
fi

for unit in hone-quant.service hone-quant-backup.service hone-quant-backup.timer; do
  install -m 0644 "$deploy_dir/systemd/$unit" "/etc/systemd/system/$unit"
done
systemctl daemon-reload
systemctl enable hone-quant.service hone-quant-backup.timer >/dev/null
echo "systemd units installed and enabled (not started)"

command -v pg_dump >/dev/null || echo "note: install postgresql-client (same major version as the server) for backups"
echo "next: edit /etc/hone-quant/runtime.env, then deploy a release with scripts/deploy.sh"
