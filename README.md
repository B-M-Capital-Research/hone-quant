<p align="center">
  <img src="web/public/logo.svg" alt="HONE" width="120">
</p>

<h1 align="center">hone-quant</h1>

<p align="center">
  <strong>Automated quantitative trading built on <a href="https://github.com/B-M-Capital-Research/honeclaw">honeclaw</a>.</strong><br>
  Researchers maintain the ontology. AI agents generate and execute the strategies. Every decision is on the record.
</p>

<p align="center">
  <strong>English</strong> · <a href="README_ZH.md">简体中文</a> ·
  <a href="https://hone-claw.com">Website</a> ·
  <a href="https://github.com/B-M-Capital-Research/honeclaw">honeclaw</a> ·
  <a href="#contact">Contact</a>
</p>

---

hone-quant turns honeclaw's industry research into a disciplined, automated portfolio process for
US equities. honeclaw's ontology defines the AI-infrastructure universe (ten sectors, 64
companies); hone-quant sizes positions on it, generates a trading plan twice every trading day,
executes it on a paper account and keeps a complete, auditable history of strategies, plans,
orders, fills and manual decisions. The interface is bilingual (English / 简体中文) and shows
Singapore and New York time side by side.

> **Paper trading only.** hone-quant contains no brokerage integration and cannot place real
> orders. Nothing here is investment advice.

**Contents:** [Backtest results](#backtest-results) ·
[Researchers, ontology and agents](#researchers-ontology-and-agents) · [Highlights](#highlights) ·
[Screenshots](#screenshots) · [Quick start](#quick-start) · [A day with hone-quant](#a-day-with-hone-quant) ·
[Deploying](#deploying-on-google-compute-engine) · [Configuration](#configuration) ·
[Command line](#command-line) · [Status](#status) · [Repository layout](#repository-layout) ·
[Development](#development) · [Documentation](#documentation) · [Contact](#contact)

## Backtest results

![Backtests on hone-claw.com/quant: the sector-first risk budget over three years and one year, with total return, annualised return, Sharpe ratio, maximum drawdown and excess return over QQQ](docs/screenshots/backtests-zh.png)

The default strategy, **sector-first risk budget**, backtested on real market data in the
production instance at hone-claw.com/quant:

| | **3 years** | **1 year** |
| --- | ---: | ---: |
| Period | 2023-10-05 → 2026-10-05 | 2025-10-05 → 2026-10-05 |
| Total return | **+535.88%** | **+70.69%** |
| Annualised return | +85.24% | +71.01% |
| Sharpe ratio | 2.10 | 1.59 |
| Maximum drawdown | −28.01% | −24.57% |
| Excess return vs QQQ | **+421.46%** | +45.68% |

Both runs use [Financial Modeling Prep](https://financialmodelingprep.com) daily prices adjusted
for splits and dividends, an opening and a pre-close plan every trading day, and slippage,
commissions and SEC fees. They run on the same strategy engine, rebalancer and cost model as the
live paper account. The strategy is the built-in default preset (`sector_risk_budget`, described in
[docs/methodology.md](docs/methodology.md)), so anyone with an FMP key can reproduce these runs
under **Backtests → New backtest**.

**Read these numbers with their limits.** The app prints the same list next to every backtest.

- **Survivorship bias.** The universe is today's ontology membership. Companies that failed or were
  never selected during the period are not in the sample, and 2023–2026 was an exceptional period
  for AI infrastructure. Results are likely optimistic.
- **No look-ahead.** Each rebalance uses closes up to the previous session plus the price at order
  time. Companies without enough history are not allocated.
- **A backtest is not a track record.** The live paper account started on 2026-10-05. Past and
  simulated performance do not guarantee future results.

## Researchers, ontology and agents

```mermaid
flowchart LR
  R["Researchers<br/>maintain the ontology"] --> O["honeclaw ontology<br/>10 sectors · 64 companies"]
  O --> A["AI agents<br/>generate and test strategies"]
  A --> E["hone-quant engine<br/>two plans every trading day"]
  E --> P["Paper account<br/>orders · fills · audit log"]
  P -- "performance, attribution, alerts" --> R
```

The results above come from a division of labour in which each side does what it is best at.

- **Professional researchers maintain and operate the ontology.** In honeclaw they model the
  AI-infrastructure supply chain: AI chips, memory and storage, optical interconnect, power, data
  centres and more. Each company has a role, and the model traces how demand reaches it. The
  ontology decides *what* hone-quant may hold. hone-quant never picks stocks. Membership changes
  arrive as a diff that an operator reviews before it applies.
- **AI agents generate and execute strategies on that foundation, rigorously.** A strategy
  follows the ontology's structure: it sets sector budgets first (risk-based, momentum- and
  trend-aware), then sizes companies within each sector by inverse volatility under a 5%
  single-name cap, and holds cash when breadth weakens. Strategies are backtested on the same
  engine that trades. Each one is stored as an immutable version with an activation history, and
  every plan explains why each target weight is what it is. AI coding agents also wrote this
  repository against a written brief, with more than 300 automated tests and checks.
- **The engine adds discipline that does not drift.** Two plans every US trading day, each with
  a review window. Executions check quote freshness and price deviation, and a turnover cap and
  tolerance bands stop needless trades. There is no leverage. People can approve, cancel or pause
  at any time, and every action is written to the audit log.

The ontology gives the strategy structure that prices alone cannot: which companies belong
together and how much risk each part of the supply chain should carry. The agents apply that
structure the same way every day, without fatigue or second-guessing. The engine makes sure what
was decided is exactly what gets traded.

## Highlights

- **All assets at a glance.** The overview's main chart shows every company's move over the
  chosen period (1 day to 1 year) as one candle, grouped by sector, with current and target
  weights underneath. One click opens a company's own candles with moving averages, volume and
  this account's fills.
- **Two plans a day, automatically.** An opening plan (30 minutes after the open) and a
  pre-close plan (three hours before the close), each with a review window. Run fully
  automatically, require approval, or pause. You can cancel a plan, remove single orders, or skip
  the rest of the day at any time.
- **A transparent strategy.** Every plan shows exactly why each target is what it is. Strategies
  are immutable versions with an activation history. Four presets are included.
- **Realistic paper execution.** Fresh-quote and price-deviation checks, slippage, commissions
  and SEC fees, whole shares, no leverage, dividends and splits applied.
- **Research.** Backtests run the same engine on up to ten years of adjusted daily history
  against SPY, QQQ, SMH and the equal-weight universe. They report full risk and return statistics
  with clear bias warnings. Live performance analysis includes attribution by sector and company.
- **Notifications and reminders** in Chinese or English: in-app, Telegram, Feishu, WeCom, Slack,
  Discord, signed webhooks and e-mail, with quiet hours, daily summaries, weekly reports and
  risk alerts.
- **Built to be operated.** One binary with the UI embedded, PostgreSQL in its own schema
  (shareable with honeclaw), hardened systemd deployment for Google Compute Engine, backups,
  an audit log and CI.

## Screenshots

Captured from the production instance at hone-claw.com/quant with real market data, in the
interface's Chinese mode; every page is also available in English (globe icon, top right).

![Overview: every company's move as one candle, grouped by sector, with current and target weights, today's plans and sector allocation](docs/screenshots/overview-zh.png)

| | |
| --- | --- |
| ![Orders and fills](docs/screenshots/trades-zh.png)<br>**Orders and fills**: every order the plans generated, its fill and its weight change, exportable as CSV. | ![Trading plan](docs/screenshots/plan-zh.png)<br>**A trading plan**: generation, review window and execution, then every order with its weight before → target → after. |
| ![Backtest report](docs/screenshots/backtest-zh.png)<br>**A backtest report** on the same engine: return, risk and benchmark statistics with bias warnings. | ![Universe](docs/screenshots/universe-zh.png)<br>**The universe** from the honeclaw ontology: sectors, members, trading restrictions and ontology updates. |
| ![Settings](docs/screenshots/settings-zh.png)<br>**Settings**, here the plan schedule with a timetable in both time zones. | ![Notifications](docs/screenshots/notifications-zh.png)<br>**Notifications and reminders**, in-app and on seven external channels. |

## Quick start

There are two ways to run hone-quant on your own machine. Both start in **demo mode**, a
deterministic synthetic market that needs no API key and is clearly labelled in the UI, with a
US$1,000,000 paper account. Both were run from scratch (fresh clone, empty database) on 2026-10-05.

### Option A — Docker Compose

Requirements: Docker Engine with Compose v2.

```bash
git clone https://github.com/B-M-Capital-Research/hone-quant.git
cd hone-quant
docker compose up --build        # the first build compiles the server: allow several minutes
```

Open <http://127.0.0.1:8090> and sign in as **admin** / **change-this-password**.

- To choose your own first password: `HONE_QUANT_ADMIN_PASSWORD='a-long-passphrase' docker compose up --build`
  (at least 10 characters; it is only used while no user exists yet).
- Ctrl-C stops the stack and keeps the data; `docker compose down -v` removes containers and data.
- PostgreSQL is published on `127.0.0.1:5434`, so it does not clash with a local instance.

### Option B — from source

Requirements: Rust 1.88+, Bun 1.3+, PostgreSQL 14+.

```bash
git clone https://github.com/B-M-Capital-Research/hone-quant.git
cd hone-quant

# 1. A role and a database, once. The file goes in on stdin because the postgres user
#    usually cannot read files in your home directory.
sudo -u postgres psql -v pw="'hone_quant_dev'" < deploy/postgres/setup.sql

# 2. A minimal .env (read from the directory you start hone-quant in).
cat > .env <<'EOF'
HONE_QUANT_DATABASE_URL=postgres://hone_quant:hone_quant_dev@127.0.0.1:5432/hone_quant
HONE_QUANT_MARKET_DATA=demo
HONE_QUANT_ADMIN_PASSWORD=change-this-password
EOF

# 3. Build the UI (it is embedded into the binary at compile time), then build and run the server.
(cd web && bun install && bun run build)
cargo run --release -p quant-server -- serve
```

Open <http://127.0.0.1:8090> and sign in as **admin** / **change-this-password**. On the first
start hone-quant applies its migrations, creates the administrator and the paper account,
activates the default strategy and loads ten years of daily history. In demo mode this takes
seconds.

### Use real market data (FMP)

hone-quant reads prices from [Financial Modeling Prep](https://financialmodelingprep.com), the
provider honeclaw uses. Change two lines in `.env`:

```bash
HONE_QUANT_MARKET_DATA=fmp
HONE_QUANT_FMP_API_KEY=your-key    # or HONE_QUANT_HONECLAW_CONFIG=/path/to/honeclaw/config.yaml
```

Then check the key and start again:

```bash
cargo run --release -p quant-server -- fmp-check   # every endpoint should report OK
cargo run --release -p quant-server -- serve       # the first start downloads 10 years of history
```

With Docker: `HONE_QUANT_MARKET_DATA=fmp HONE_QUANT_FMP_API_KEY=your-key docker compose up --build`.

Demo and real data live in separate schemas (`hone_quant_demo` and `hone_quant`), so switching
never mixes them. The real account starts fresh, including its users: keep
`HONE_QUANT_ADMIN_PASSWORD` in `.env` for that first start, then remove it. To reproduce the
backtests above, open **Backtests → New backtest** and choose the sector-first risk budget over
three years or one year.

### Rehearse a trading day outside US market hours

To watch plans being generated and executed at any time of day, start the demo with a simulated
clock. Give it a schema of its own so the simulated day does not mix with real-clock history:

```bash
HONE_QUANT_DB_SCHEMA=hone_quant_rehearsal HONE_QUANT_DEV_CLOCK=2026-10-05T13:55:00Z \
  cargo run --release -p quant-server -- serve
```

The clock starts at 09:55 New York time (21:55 in Singapore) and runs in real time. The opening
plan is generated at 10:00 and executes after its ten-minute review window. Demo mode only.

### Your first ten minutes

1. **Overview**: the candle board shows every company's move for the chosen period, grouped by
   sector, with current (grey) and target (red tick) weights. Click a candle for that company's
   chart; *Single asset* steps through the universe.
2. **Strategy**: *How it works* explains the active version with its own numbers. *Tune & preview*
   shows what a parameter change would do to today's targets before you save it as a new version.
3. **Backtests → New backtest**: run the active strategy or a preset over one to ten years against
   SPY, QQQ, SMH and the equal-weight universe.
4. **Settings**: plan times, automatic or approval mode, execution costs, risk alerts, and
   *Notifications*, where you can add a Telegram, Feishu, WeCom, Slack, Discord, webhook or e-mail
   channel and send a test message.
5. **Trading plans**: you are notified when a plan is generated. Open it to see every order and
   the reasoning behind every target. During the review window you can cancel the plan or remove
   single orders.

## A day with hone-quant

Default times in Singapore during US daylight-saving time (one hour later in the northern winter).
The interface always shows both time zones, and every time is configurable.

| SGT | What happens | What you do |
| --- | --- | --- |
| 20:00 | Daily history and corporate actions refreshed | — |
| 21:00 | Pre-open briefing: today's plan times and automation mode | Skip a slot if you want no trading today |
| 22:00 | Opening plan generated and notified | Open it: orders, turnover, costs and the reasoning behind every target |
| 22:10 | The plan executes after its review window (automatic mode) | Or approve it yourself in approval mode; cancel if something looks wrong |
| 01:00 | Pre-close plan (quiet hours hold back non-critical messages) | Usually nothing: automatic mode with risk alerts covers the night |
| 04:15 | Close: end-of-day snapshot, then the daily summary at 04:20 (weekly report on Saturdays) | Read the summary in the morning |

Slots missed while the service is down are recorded as missed and never executed late.
Early-close days run only the opening plan.

## Deploying on Google Compute Engine

The full runbook is **[docs/deployment-gce.md](docs/deployment-gce.md)**
([中文](docs/deployment-gce.zh.md)). In short:

1. **Database.** On the PostgreSQL instance honeclaw already uses, create a role and a dedicated
   database: `sudo -u postgres psql -v pw="'<strong password>'" < deploy/postgres/setup.sql`.
   hone-quant keeps everything in its own schema and never touches honeclaw's tables.
2. **Release.** Push a `vX.Y.Z` tag to run the *Release* GitHub Actions workflow, or build locally
   with `scripts/build-release.sh` → `dist/hone-quant-<version>-<revision>-linux-x86_64.tar.gz`.
3. **Host.** Copy the tarball to the VM, run `sudo scripts/install-host.sh` (system user,
   directories, hardened systemd units, backup timer) and fill in `/etc/hone-quant/runtime.env`:
   database URL, FMP key, first administrator.
4. **Deploy.** `sudo scripts/deploy.sh <tarball>` migrates, switches atomically, health-checks and
   rolls back on failure. Then `sudo /opt/hone-quant/current/scripts/hq.sh fmp-check`.
5. **Access.** No public port is needed:
   `gcloud compute ssh <vm> --zone <zone> --tunnel-through-iap -- -N -L 8090:127.0.0.1:8090`, then
   open <http://127.0.0.1:8090>. HTTPS on your own domain is optional (`deploy/caddy/`).

A daily timer dumps the `hone_quant` schema (newest 30 kept). Back up
`/var/lib/hone-quant/secret.key` as well: it seals the saved notification-channel credentials.

### Served at hone-claw.com/quant

The production instance runs on honeclaw's VM under **https://hone-claw.com/quant** and has no
accounts of its own. hone-quant checks the visitor's hone-claw.com session on the server and
admits administrators only. A Cloudflare Worker routes `/quant*` to the origin with a shared
origin token. The runbook and rollback steps are in
**[docs/deploy-hone-claw-quant.md](docs/deploy-hone-claw-quant.md)**.

## Configuration

Deployment facts are environment variables: `.env` for local runs, `/etc/hone-quant/runtime.env`
in production. Everything you tune while the app runs (plan times, automation mode, costs, risk
alerts, notification channels, reminders, strategy versions) is edited in the web UI, stored in
PostgreSQL and recorded in the audit log.

| Variable | Default | Purpose |
| --- | --- | --- |
| `HONE_QUANT_DATABASE_URL` | — | PostgreSQL connection; alternatively `HONE_QUANT_PG_*`, or honeclaw's `DATABASE_URL` / `HONE_POSTGRES_*` |
| `HONE_QUANT_DB_SCHEMA` | `hone_quant` (`hone_quant_demo` in demo mode) | Schema that holds every hone-quant table |
| `HONE_QUANT_MARKET_DATA` | `fmp` | `fmp` (real data) or `demo` (synthetic) |
| `HONE_QUANT_FMP_API_KEY` | — | FMP key; `HONE_QUANT_FMP_API_KEYS` rotates several, `HONE_QUANT_HONECLAW_CONFIG` reuses honeclaw's |
| `HONE_QUANT_BIND` | `127.0.0.1:8090` | Listen address (keep it on loopback and use a tunnel or reverse proxy) |
| `HONE_QUANT_STATE_DIR` | `./data` | Holds `secret.key`, which seals notification credentials |
| `HONE_QUANT_ADMIN_USER` / `HONE_QUANT_ADMIN_PASSWORD` | `admin` / — | First administrator, created only while no user exists |
| `HONE_QUANT_INITIAL_CASH` | `1000000` | Starting cash of the paper account |
| `HONE_QUANT_PUBLIC_URL` | — | Public HTTPS URL behind a reverse proxy: links in notifications, and the only `Origin` accepted for writes |
| `HONE_QUANT_BASE_PATH` | — | Serve under a path such as `/quant` (build the web UI with the same value) |
| `HONE_QUANT_AUTH_MODE` | `local` | `honeclaw`: no local accounts; honeclaw administrators only, checked on the server (`HONE_QUANT_HONECLAW_*`) |
| `HONE_QUANT_ORIGIN_TOKEN` | — | Secret the proxy in front must send as `X-Hone-Quant-Origin-Token`; anything else gets 404 |
| `HONE_QUANT_DEV_CLOCK` | — | Demo only: start the clock at this instant (RFC 3339) |

Every variable is documented in [`deploy/runtime.env.example`](deploy/runtime.env.example).

## Command line

```text
hone-quant serve                        web UI, API, scheduler and paper broker (the default)
hone-quant migrate                      apply database migrations and exit
hone-quant user add <name> [--role admin|viewer]
hone-quant user passwd <name>           set a new password and sign out that user's sessions
hone-quant user list
hone-quant fmp-check [--symbol NVDA]    probe every FMP endpoint hone-quant uses
hone-quant sync [--full]                fetch market data now
hone-quant universe sync [--apply]      compare honeclaw's ontology with the database
hone-quant universe build --edits <honeclaw>/data/industry_map/edits.json
```

From source, prefix with `cargo run --release -p quant-server --`. On a deployed host, run them
with the service's user and environment: `sudo /opt/hone-quant/current/scripts/hq.sh <command>`.

## Status

*As of 2026-10-06.*

- **In production** at https://hone-claw.com/quant since 2026-10-05, for hone-claw.com
  administrators. It runs on honeclaw's PostgreSQL (its own role and database) with real FMP data:
  ten years of daily bars for all 67 symbols.
- **Feature-complete for its brief.** That covers the ontology universe, price-only weighting, two
  plans per trading day, automatic paper execution, and manual intervention (cancel, remove
  orders, skip the day, approval mode, pause, switch strategy version). It also covers
  notifications and reminders, configurable settings, a complete audit trail, backtesting,
  performance analysis, a Chinese/English UI and GCE deployment tooling.
- **Tested.** 82 engine tests and 201 server tests, the latter including PostgreSQL integration
  tests, an FMP client against a mock FMP server, notification channels and the honeclaw sign-in.
  Also 22 web unit tests and 5 browser end-to-end tests, 15 tests for the Cloudflare Worker, and
  22 checks of its deploy script against a mock Cloudflare API. `cargo clippy` is clean.
- **Still young.** The live paper account started on 2026-10-05. Annualised live figures need 21
  trading days to show and about 63 to be reliable, so use backtests meanwhile. The external
  notification channels have only been tested against local mocks so far.

**Out of scope by design**

- **Live trading.** There is no broker integration and no setting that enables one.
- **Stock selection.** The universe comes from honeclaw's ontology; hone-quant only sizes positions.
- **Intraday signals.** Strategies use daily closes plus the current quote; backtests use daily bars.

## Repository layout

```text
crates/quant-core     strategy, rebalancer, costs, schedule, metrics and backtester: pure Rust, no I/O
crates/quant-server   axum API, scheduler, paper broker, FMP client, notifications, PostgreSQL migrations
web/                  SolidJS + ECharts UI (zh/en), embedded into the server binary
config/               bundled universe snapshot built from the honeclaw ontology
deploy/               systemd units, runtime.env template, PostgreSQL setup, Caddy example and
                      hone-claw.com/quant route, Cloudflare Worker (cloudflare/quant-proxy)
scripts/              release build, host install, deploy with rollback, backups, hq.sh wrapper
docs/                 methodology, architecture, GCE runbook (en/zh), hone-claw.com/quant runbook, screenshots
```

## Development

```bash
# Rust (integration tests need a PostgreSQL database they can create schemas in)
export HONE_QUANT_TEST_DATABASE_URL=postgres://hone_quant:…@127.0.0.1:5432/hone_quant
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace

# Web: typecheck, unit tests, production build
(cd web && bun run typecheck && bun run test && bun run build)

# UI development with hot reload: the API in one terminal, the UI on http://127.0.0.1:5173 in another
cargo run -p quant-server
(cd web && bun run dev)

# Browser end-to-end tests and screenshots against a running app (demo data recommended)
(cd web && HONE_QUANT_E2E_URL=http://127.0.0.1:8090 HONE_QUANT_E2E_PASSWORD=… bun run test:e2e)
(cd web && HONE_QUANT_SHOT_PASSWORD=… node scripts/screenshot.mjs --base http://127.0.0.1:8090 --pages /,/plans --locales zh,en --themes light,dark)
```

The bundled universe is regenerated from honeclaw's ontology with
`hone-quant universe build --edits <honeclaw>/data/industry_map/edits.json`.

## Documentation

- [Strategy and trading methodology](docs/methodology.md) ([中文](docs/methodology.zh.md))
- [Architecture](docs/architecture.md)
- [Deployment on Google Compute Engine](docs/deployment-gce.md) ([中文](docs/deployment-gce.zh.md))
- [hone-claw.com/quant: production deployment](docs/deploy-hone-claw-quant.md) and its
  [Cloudflare Worker](deploy/cloudflare/quant-proxy/README.md)
- Configuration reference: [`deploy/runtime.env.example`](deploy/runtime.env.example)

## Contact

We are an investment research organization based in Singapore, and the team behind
[honeclaw](https://github.com/B-M-Capital-Research/honeclaw) and hone-quant. If these open-source
projects interest you, write to us. You might want to use them, build on them or work with us.
We also offer community consultation, investment research guidance and related services.

- **E-mail:** [contact@honeclaw.app](mailto:contact@honeclaw.app)
- **Website:** [hone-claw.com](https://hone-claw.com)

Issues and pull requests are welcome in this repository.

## License

[MIT](LICENSE), like honeclaw. Nothing in this repository is investment advice.
