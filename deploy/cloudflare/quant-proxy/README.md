# hone-quant-proxy (Cloudflare Worker)

Serves **https://hone-claw.com/quant** from hone-quant on honeclaw's origin. The full runbook,
with the origin side and rollback, is [docs/deploy-hone-claw-quant.md](../../../docs/deploy-hone-claw-quant.md).

| | |
| --- | --- |
| Worker | `hone-quant-proxy`, [`src/index.js`](src/index.js) (ES module, no dependencies) |
| Route | `hone-claw.com/quant*` on zone `hone-claw.com`. One pattern: route matching includes the query string, so only a trailing `*` also covers `/quant?x=1`. The Worker passes `/quantum` and similar paths back to the zone untouched. |
| Var | `QUANT_ORIGIN_URL` = `https://<origin-host>`, the bare https origin of the VM. Kept out of the repository: pass it at deploy time. |
| Secret | `QUANT_ORIGIN_TOKEN` = `HONE_QUANT_ORIGIN_TOKEN` from `/etc/hone-quant/runtime.env` on the origin |
| DNS | No change. hone-claw.com stays with the Pages project; the route runs in front of it for `/quant*` only. |
| workers.dev / previews | Off: the Worker is reachable through the route only. |

What it does for `/quant` and `/quant/…`: redirects plain http to https; forwards the request
(method, path, query, body) to the origin with `X-Hone-Quant-Origin-Token` set (a client-supplied
one is replaced) and only the `hone_web_session` cookie; passes the response through unchanged,
redirects included; never lets the edge cache anything except the content-hashed
`/quant/assets/*`. Without a valid configuration it answers `503 hone_quant_proxy_not_configured`;
when the origin is unreachable, `502 hone_quant_origin_unreachable`. Signing in is checked by
hone-quant itself (honeclaw administrators only), never by the Worker.

## Get the origin token

On the GCE VM (it never needs to leave the VM except into the Worker secret):

```sh
sudo sed -n 's/^HONE_QUANT_ORIGIN_TOKEN=//p' /etc/hone-quant/runtime.env
```

## Deploy — dashboard

1. **Workers & Pages** → **Create** → **Create Worker** → name `hone-quant-proxy` → **Deploy**.
2. **Edit code** → replace everything with [`src/index.js`](src/index.js) → **Deploy**.
3. **Settings → Variables and Secrets → Add**
   - Type *Text*, name `QUANT_ORIGIN_URL`, value `https://<origin-host>`
   - Type *Secret*, name `QUANT_ORIGIN_TOKEN`, value: the origin token

   → **Deploy**.
4. **Settings → Domains & Routes**: turn **workers.dev** and **Preview URLs** off.
5. **Settings → Domains & Routes → Add → Route**: zone `hone-claw.com`, route
   `hone-claw.com/quant*` → **Add route**.

## Deploy — wrangler

```sh
cd deploy/cloudflare/quant-proxy
export CLOUDFLARE_ACCOUNT_ID=... QUANT_ORIGIN_URL=https://<origin-host>
npx wrangler@4 deploy --var QUANT_ORIGIN_URL:"$QUANT_ORIGIN_URL"
ssh <origin> "sudo sed -n 's/^HONE_QUANT_ORIGIN_TOKEN=//p' /etc/hone-quant/runtime.env" \
  | npx wrangler@4 secret put QUANT_ORIGIN_TOKEN
```

Between the two commands the Worker answers 503 (it fails closed without the secret).
`keep_vars` keeps the origin URL when a later deploy omits `--var`.

## Deploy — Cloudflare API ([`deploy.sh`](deploy.sh))

Needs bash, curl and python3, and an API token with **Account › Workers Scripts › Edit** and
**Zone (hone-claw.com) › Workers Routes › Edit**.

```sh
export CLOUDFLARE_API_TOKEN=... CLOUDFLARE_ACCOUNT_ID=... CLOUDFLARE_ZONE_ID=...
export QUANT_ORIGIN_URL=https://<origin-host>
./deploy.sh --dry-run          # token status, existing /quant routes, what would change
ssh <origin> "sudo sed -n 's/^HONE_QUANT_ORIGIN_TOKEN=//p' /etc/hone-quant/runtime.env" | ./deploy.sh
```

It uploads the script (keeping existing secrets), sets the secret from stdin, turns workers.dev and
previews off, creates the route last and then checks the public URL. A later `./deploy.sh` without
stdin redeploys the code and keeps the secret.

## Check

```sh
curl -sS https://hone-claw.com/quant/api/health                                       # {"db":true,"ok":true,...}
curl -sS -o /dev/null -w '%{http_code}\n' https://hone-claw.com/quant/api/auth/me      # 401 (no session)
curl -sS -o /dev/null -w '%{http_code} %{redirect_url}\n' https://hone-claw.com/quant  # 308 .../quant/
```

Then sign in at https://hone-claw.com with an administrator account and open
https://hone-claw.com/quant/.

## Roll back

Delete the route (dashboard: **Settings → Domains & Routes**, or `./deploy.sh --rollback`): `/quant`
is served by the Pages site again within seconds. `./deploy.sh --rollback --delete-worker` also
removes the Worker.

## Tests

```sh
bun test                 # the Worker: routing, headers, cookies, cache policy, failures (15 tests)
bash test/deploy.test.sh # deploy.sh against a mock Cloudflare API (22 checks, no network)
```
