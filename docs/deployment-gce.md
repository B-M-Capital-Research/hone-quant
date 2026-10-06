# Deploying hone-quant on Google Compute Engine

This runbook installs hone-quant on a Linux VM in Google Compute Engine (GCE), next to honeclaw
and sharing its PostgreSQL instance. 中文版：[deployment-gce.zh.md](deployment-gce.zh.md).
The production instance at hone-claw.com/quant (honeclaw administrators only, behind a
Cloudflare Worker) has its own runbook: [deploy-hone-claw-quant.md](deploy-hone-claw-quant.md).

hone-quant is a single binary (the web UI is embedded) managed by systemd. It listens on
`127.0.0.1:8090` only; you reach it through an IAP/SSH tunnel (recommended) or an HTTPS reverse
proxy. **It only ever trades a simulated paper account** — the code base contains no brokerage
integration, and no setting can turn one on.

```
 your laptop ──IAP tunnel──▶ VM: hone-quant (127.0.0.1:8090, systemd)
                                  ├─▶ PostgreSQL (shared with honeclaw; schema hone_quant)
                                  ├─▶ financialmodelingprep.com (market data, HTTPS)
                                  └─▶ Telegram / Feishu / WeCom / Slack / Discord / email (notifications)
```

## 1. Requirements

| Item | Recommendation |
| --- | --- |
| VM | e2-small (2 vCPU, 2 GB) or larger; Debian 12 or Ubuntu 22.04/24.04 (x86_64) |
| Disk | 10 GB free for the database (≈10 years of daily bars for ~70 symbols is well under 1 GB), releases and backups |
| PostgreSQL | 14 or newer; the instance honeclaw already uses is fine |
| Network | Outbound HTTPS. No inbound port is needed when you use the IAP tunnel |
| Market data | A Financial Modeling Prep key with US quotes, daily history and intraday charts |
| Clock | GCE images sync time via the metadata server; keep it that way (plan times are clock-driven) |

The host's time zone does not matter: market logic always uses `America/New_York`, and the UI
shows Singapore time (configurable) next to New York time.

## 2. PostgreSQL

hone-quant keeps every object in its own schema (`hone_quant`) and never reads or writes
honeclaw's tables. Create a role once, as a PostgreSQL superuser:

```bash
sudo -u postgres psql -v pw="'<a strong password>'" < deploy/postgres/setup.sql
```

The file is fed on standard input because the `postgres` user usually cannot read files in
your home directory (`-f` would fail with "Permission denied").
`setup.sql` creates a dedicated `hone_quant` database on the shared instance (recommended).
It also documents the alternative of a `hone_quant` schema inside honeclaw's database. Tables are
created automatically on first start by checksummed, versioned migrations.

If the VM's environment already exports honeclaw's `DATABASE_URL` or `HONE_POSTGRES_*`, hone-quant
can borrow them (its objects still go into the `hone_quant` schema), but an explicit
`HONE_QUANT_DATABASE_URL` with its own role is clearer and easier to audit.

## 3. Build a release

Releases are built by the **Release** GitHub Actions workflow on Ubuntu 22.04 (so the binary runs
on Debian 12 and Ubuntu 22.04+): push a tag `vX.Y.Z` to publish a GitHub release, or run the
workflow manually and download the artifact. To build locally instead (needs Rust and Bun):

```bash
scripts/build-release.sh
# → dist/hone-quant-<version>-<revision>-linux-x86_64.tar.gz and .sha256
```

## 4. First installation

Copy the release to the VM and prepare the host (one time):

```bash
gcloud compute scp dist/hone-quant-*.tar.gz* <vm>:/tmp/ --zone <zone> --tunnel-through-iap
gcloud compute ssh <vm> --zone <zone> --tunnel-through-iap

# on the VM
cd /tmp && tar -xzf hone-quant-*.tar.gz && cd hone-quant-*/
sudo scripts/install-host.sh            # user, directories, systemd units, runtime.env template
sudo apt-get install -y postgresql-client curl   # pg_dump for backups, curl for health checks
sudoedit /etc/hone-quant/runtime.env
```

In `runtime.env` set at least:

- `HONE_QUANT_DATABASE_URL` — the role and database from step 2;
- `HONE_QUANT_FMP_API_KEY` (or `HONE_QUANT_HONECLAW_CONFIG` pointing at honeclaw's `config.yaml`
  to reuse its `fmp:` keys);
- `HONE_QUANT_ADMIN_USER` / `HONE_QUANT_ADMIN_PASSWORD` for the first administrator (remove the
  password after the first sign-in).

Every variable is documented in `deploy/runtime.env.example`. Then deploy and start:

```bash
sudo scripts/deploy.sh /tmp/hone-quant-<version>-<revision>-linux-x86_64.tar.gz
sudo /opt/hone-quant/current/scripts/hq.sh fmp-check     # every endpoint should report OK
```

On first start hone-quant applies migrations, loads the universe (10 sectors, 64 companies from the
honeclaw ontology), creates the paper account (US$1,000,000 by default), activates the default
strategy and downloads ten years of daily history (a few minutes).

## 5. Access

**IAP tunnel (recommended, no public port).** Allow IAP's range to reach SSH once:

```bash
gcloud compute firewall-rules create allow-iap-ssh --network <network> \
  --direction INGRESS --action allow --rules tcp:22 --source-ranges 35.235.240.0/20
```

Then, from your laptop:

```bash
gcloud compute ssh <vm> --zone <zone> --tunnel-through-iap -- -N -L 8090:127.0.0.1:8090
```

and open <http://127.0.0.1:8090>. Close the tunnel with Ctrl-C.

**HTTPS on your own domain (optional).** Put Caddy (or nginx) in front, see
`deploy/caddy/Caddyfile.example`, and set `HONE_QUANT_PUBLIC_URL=https://…` and
`HONE_QUANT_SECURE_COOKIE=true`. Prefer an additional access layer (Cloudflare Access, Google IAP
for HTTPS, or a source-IP allow-list): a private trading desk does not need to be public.

## 6. Operating

| Task | Command |
| --- | --- |
| Status / logs | `systemctl status hone-quant` · `journalctl -u hone-quant -f` |
| Health | `curl -s 127.0.0.1:8090/api/health` → `{"ok":true,"db":true,"version":…,"revision":…}` |
| Add an operator | `sudo /opt/hone-quant/current/scripts/hq.sh user add alice --role viewer` |
| Reset a password | `sudo …/hq.sh user passwd alice` (signs out their sessions) |
| Market data now | `sudo …/hq.sh sync` (`--full` re-downloads the whole history) |
| Universe update | Universe page → "Check for updates", or `sudo …/hq.sh universe sync --apply` |
| Upgrade | `sudo scripts/deploy.sh <new tarball>` (migrates, switches, health-checks, rolls back on failure) |
| Roll back | `sudo ln -sfn /opt/hone-quant/releases/<previous> /opt/hone-quant/current && sudo systemctl restart hone-quant` |

Day to day, everything else — plan times, automation mode, costs, risk alerts, notification
channels, reminders, strategy versions — is managed in the web UI, and every change is recorded in
the audit log.

**What runs when (New York time).** Pre-open data sync 08:00; opening plan 10:00 (open + 30 min);
pre-close plan 13:00 (close − 3 h); after each plan a review window (10 min by default) before
automatic execution; post-close sync and end-of-day snapshot 16:15; daily summary afterwards.
Early-close days (13:00 close) run only the opening plan. Slots missed while the service is down
are recorded as missed — they are never executed late.

## 7. Backups

`hone-quant-backup.timer` runs `scripts/backup.sh` daily at 22:30 UTC (after the US close) and
keeps the newest 30 dumps of the `hone_quant` schema in `/var/backups/hone-quant`. Also back up
`/var/lib/hone-quant/secret.key` — without it, saved notification-channel credentials cannot be
decrypted (everything else still works; the channels just need to be re-entered).

```bash
sudo systemctl start hone-quant-backup.service      # run one now
sudo ls -l /var/backups/hone-quant
# restore (stops the app first)
sudo systemctl stop hone-quant
sudo -u hone-quant pg_restore --clean --if-exists --no-owner -d "<database url>" <dump>
sudo systemctl start hone-quant
```

GCE persistent-disk snapshots are a good second layer.

## 8. Troubleshooting

| Symptom | Check |
| --- | --- |
| Service fails at start | `journalctl -u hone-quant -n 100` — configuration errors name the variable to fix |
| "cannot create schema" | The role needs `CREATE` on the database, or pre-create the schema (setup.sql, layout B) |
| Data stale alerts | `hq.sh fmp-check`; FMP plan limits (HTTP 402/403) and rate limits show up there |
| Plans skipped as "missed" | The service was down or the clock was wrong at plan time; see Settings → Data → jobs |
| Plans "expired" | Approval mode and nobody approved before the deadline — switch to automatic or approve earlier |
| Notification not delivered | Settings → Notifications → channel → "Send test"; quiet hours defer non-critical messages |
