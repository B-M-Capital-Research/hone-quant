# hone-claw.com/quant

hone-quant runs on honeclaw's GCE VM and is served under **https://hone-claw.com/quant**. There
are no hone-quant accounts: whoever is signed in to hone-claw.com **as an administrator** can use
it; everyone else is turned away. It only ever trades a simulated paper account.

For a general installation (IAP tunnel, local accounts) see [deployment-gce.md](deployment-gce.md).

## How a request flows

```
browser  https://hone-claw.com/quant/...   (hone_web_session cookie: Path=/, HttpOnly, Secure, SameSite=Strict)
   │
   ▼  Cloudflare, zone hone-claw.com
Worker hone-quant-proxy, route hone-claw.com/quant*
   │  adds X-Hone-Quant-Origin-Token, forwards only the hone_web_session cookie, no edge caching
   ▼
https://<origin-host>/quant/...          Caddy on the VM: /quant, /quant/* -> 127.0.0.1:8090
   ▼
hone-quant (systemd, 127.0.0.1:8090)
   │  404 unless the origin token matches · strips /quant · CSRF checks on every write
   │  each API call: cookie -> GET https://hone-claw.com/api/public/auth/me
   │     200 and user.is_admin == true -> admitted (verdict cached 30 s)
   │     200, not an administrator     -> 403
   │     401 / 403                     -> 401, signed out
   │     anything else                 -> 503 (fails closed)
   ├─▶ PostgreSQL 17 on the VM: database hone_quant (role hone_quant), schema hone_quant
   └─▶ financialmodelingprep.com with honeclaw's FMP key (at most 60 requests/min)
```

The browser never sees the origin host: hone-quant's redirects are path-only.

## What is deployed

| Piece | Where |
| --- | --- |
| Release | `/opt/hone-quant/current` → `releases/hone-quant-<version>-<revision>-linux-x86_64`; `GET /quant/api/health` reports the running revision |
| Service | `hone-quant.service` (enabled; hardened unit), `hone-quant-backup.timer` daily 22:30 UTC → `/var/backups/hone-quant` |
| Configuration | `/etc/hone-quant/runtime.env` (root:hone-quant, 0640): base path `/quant`, public URL `https://hone-claw.com/quant`, `HONE_QUANT_AUTH_MODE=honeclaw`, origin token, database URL, FMP key copied from honeclaw's `config.yaml`, `HONE_QUANT_FMP_RPM=60` |
| Database | Role `hone_quant` (no superuser, createdb or createrole) owning database `hone_quant` on the existing PostgreSQL 17 (`127.0.0.1:5432`); honeclaw's databases are untouched |
| Data | 64-asset universe; ten years of daily bars for 67 symbols (shorter for later listings) |
| State | `/var/lib/hone-quant` (`secret.key`; back it up with the database) |
| Caddy | One marked block (`# BEGIN HONE-QUANT` … `# END HONE-QUANT`) in `/etc/caddy/Caddyfile`; the file before each change is kept as `Caddyfile.bak-hone-quant-<UTC time>` |
| Cloudflare | Worker `hone-quant-proxy` on the route `hone-claw.com/quant*` (next section) |

Checked through Caddy on the origin, with and without the origin token:

| Request | Result |
| --- | --- |
| any `/quant/...` without the token, or with a wrong one | 404 |
| `/quant/api/health` (no token needed) | 200 `{"db":true,"ok":true,...}` |
| `/quant` | 308 → `/quant/` |
| `/quant/`, `/quant/plans` | 200, the app shell with `/quant/assets/...` |
| `/quant/api/meta` | 200, `auth.mode = honeclaw`, `base_path = /quant` |
| `/quant/api/auth/me`, `/quant/api/dashboard` without a session, or with a forged cookie | 401 |
| `POST` without `X-Hone-Quant-Action`, or cross-site | 403 |
| `POST /quant/api/auth/login` (local passwords) | 403 |
| honeclaw's own routes, `hone-web`, `postgresql`, `caddy` | unchanged, running |

## The Cloudflare Worker

Everything is in [`deploy/cloudflare/quant-proxy/`](../deploy/cloudflare/quant-proxy/README.md):
the Worker, `wrangler.jsonc`, an API deploy script and their tests.

1. **Origin token.** On the VM: `sudo sed -n 's/^HONE_QUANT_ORIGIN_TOKEN=//p' /etc/hone-quant/runtime.env`.
   Treat it like a password; it goes into the Worker secret and nowhere else.
2. **Worker and route**, one of:
   - **Dashboard** — Workers & Pages → Create → Create Worker `hone-quant-proxy` → Deploy → Edit
     code: paste `src/index.js` → Deploy. Settings → Variables and Secrets: Text
     `QUANT_ORIGIN_URL` = `https://<origin-host>`, Secret `QUANT_ORIGIN_TOKEN` = the token
     → Deploy. Settings → Domains & Routes: workers.dev **off**, Preview URLs **off**, then Add →
     Route: zone `hone-claw.com`, route `hone-claw.com/quant*`.
   - **wrangler** — `npx wrangler@4 deploy --var QUANT_ORIGIN_URL:https://<origin-host>` in that
     directory (with `CLOUDFLARE_ACCOUNT_ID` set), then pipe the token into
     `npx wrangler@4 secret put QUANT_ORIGIN_TOKEN`.
   - **API** — with `CLOUDFLARE_API_TOKEN`, `CLOUDFLARE_ACCOUNT_ID`, `CLOUDFLARE_ZONE_ID` and
     `QUANT_ORIGIN_URL` set, `./deploy.sh --dry-run`, then pipe the token into `./deploy.sh`.
     The token needs **Account › Workers Scripts › Edit** and **Zone (hone-claw.com) › Workers
     Routes › Edit**.

   No DNS record is added or changed.
3. **Check.**

   ```sh
   curl -sS https://hone-claw.com/quant/api/health                                       # 200 {"db":true,"ok":true,...}
   curl -sS -o /dev/null -w '%{http_code}\n' https://hone-claw.com/quant/api/auth/me      # 401
   curl -sS -o /dev/null -w '%{http_code} %{redirect_url}\n' https://hone-claw.com/quant  # 308 https://hone-claw.com/quant/
   ```

   Then sign in at https://hone-claw.com with an administrator account and open
   https://hone-claw.com/quant/. A signed-in non-administrator gets "hone-quant is limited to
   hone-claw.com administrators"; a signed-out visitor gets a link to the hone-claw.com sign-in.

### If something is off

| Symptom | Meaning |
| --- | --- |
| `503 {"error":"hone_quant_proxy_not_configured"}` | The Worker lacks `QUANT_ORIGIN_URL` or `QUANT_ORIGIN_TOKEN` (at least 32 characters) |
| `502 {"error":"hone_quant_origin_unreachable"}` | The Worker cannot reach the origin: Caddy or the VM is down |
| Plain 404 for every `/quant/...` except `/quant/api/health` | The Worker's token differs from `HONE_QUANT_ORIGIN_TOKEN`: set the secret again |
| `/quant` shows the hone-claw.com site's page | The route is missing or bound to another Worker |
| 401 in the app right after signing in | The browser sent no `hone_web_session` for hone-claw.com (signed in on another host, or the session expired) |
| 403 "limited to hone-claw.com administrators" | The account is not an administrator in honeclaw |
| 503 "the hone-claw.com sign-in cannot be verified" | hone-quant cannot reach `https://hone-claw.com/api/public/auth/me`; see `journalctl -u hone-quant` |

## Operations

```sh
systemctl status hone-quant
journalctl -u hone-quant -f                # JSON logs; secrets are never logged
curl -sS http://127.0.0.1:8090/api/health  # on the VM, no token needed
```

- **New release.** Build it for the base path — `HONE_QUANT_BASE_PATH=/quant scripts/build-release.sh`
  (the server refuses to start with a web bundle built for another path) — copy the tarball and
  its `.sha256` to the VM, unpack it and run **that release's** `scripts/deploy.sh <tarball>`. It
  migrates, switches, health-checks and rolls back on failure.
- **Backups.** `systemctl list-timers hone-quant-backup.timer` must show a next run (daily
  22:30 UTC). `deploy.sh` starts the timer after every healthy deploy; on a host installed by an
  older release, start it once with `sudo systemctl start hone-quant-backup.timer`.
- **Caddy route.** If the Caddyfile is ever restored from an older copy, run
  `sudo deploy/caddy/hone-claw-quant-route.sh` again (it is idempotent, validates before
  replacing, keeps a backup and restores it if the reload fails).
- **Rotate the origin token.** On the VM, then set the same value as the Worker secret (requests
  get 404 until both sides agree):

  ```sh
  sudo bash -c 'f=/etc/hone-quant/runtime.env; umask 077
    T="$(openssl rand -hex 32)" awk "/^HONE_QUANT_ORIGIN_TOKEN=/ { print \"HONE_QUANT_ORIGIN_TOKEN=\" ENVIRON[\"T\"]; next } { print }" "$f" >"$f.new"
    chown root:hone-quant "$f.new"; chmod 0640 "$f.new"; mv "$f.new" "$f"; systemctl restart hone-quant'
  ```

- **Rotate the database password.** `ALTER ROLE hone_quant PASSWORD ...` fed to
  `sudo -u postgres psql` on stdin, the same value in `HONE_QUANT_DATABASE_URL`, then
  `systemctl restart hone-quant`. Never pass passwords or tokens as command-line arguments:
  other users on the VM can read process arguments.
- **Start the paper account over.** Settings → Account → reset, with a new starting cash; open
  plans are cancelled and the reset is recorded in the audit log.

## Roll back

Each step stands on its own; stop wherever you want.

1. **Cloudflare** — delete the route `hone-claw.com/quant*` (Worker → Settings → Domains & Routes,
   or `./deploy.sh --rollback`; `--delete-worker` also removes the Worker). `/quant` is served by
   the Pages site again.
2. **Caddy** — `sudo deploy/caddy/hone-claw-quant-route.sh --remove`.
3. **Service** — `sudo systemctl disable --now hone-quant hone-quant-backup.timer`.
4. **Data**, only if it should go: `sudo -u postgres psql -c 'DROP DATABASE hone_quant' -c 'DROP ROLE hone_quant'`,
   then remove `/opt/hone-quant`, `/etc/hone-quant`, `/var/lib/hone-quant`, `/var/backups/hone-quant`,
   the three units in `/etc/systemd/system/` and the `hone-quant` user.

## Notes

- **Late starts.** A plan slot whose window has already passed when the service starts is
  recorded as missed and never traded late. Reset the paper account if you want a clean start.
- **Audit IPs.** Requests reach hone-quant through Cloudflare and Caddy, so the IP recorded in
  the audit log may be an edge address rather than the visitor's.
