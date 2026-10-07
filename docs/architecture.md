# Architecture

hone-quant is a Rust server with an embedded SolidJS web UI and PostgreSQL storage. It is
deliberately small: one process, one database schema, no message broker, no external job runner.

```
                ┌──────────────────────── hone-quant (one process) ────────────────────────┐
 browser ──HTTP─┤ axum API (/api/*) · SSE (/api/events) · embedded web UI (rust-embed)     │
                │                                                                          │
                │ scheduler (leader-elected) ─▶ planner ─▶ paper broker                    │
                │        │                        │            │                           │
                │        ├─▶ market data (FMP or demo)         ├─▶ notifications ─▶ Telegram │
                │        └─▶ reminders, risk checks, reports   │      Feishu, WeCom, Slack,  │
                │                                              │      Discord, webhook, mail │
                │ quant-core: calendar · strategy engine · rebalancer · costs · metrics ·   │
                │             backtester (pure, deterministic, unit-tested)                │
                └──────────────────────────────┬───────────────────────────────────────────┘
                                               │  schema hone_quant
                                         PostgreSQL (shared with honeclaw)
```

## Repository layout

| Path | Contents |
| --- | --- |
| `crates/quant-core` | Pure domain logic: NYSE calendar and plan schedule, statistics, the strategy engine, the rebalancer, the cost model, performance metrics and the daily-bar backtester. No I/O. |
| `crates/quant-server` | The `hone-quant` binary: configuration, PostgreSQL pool and migrations, stores, market data (FMP client, demo market), services (market data sync, portfolio valuation, planner, paper broker, scheduler, reminders, backtests), notifications and channels, HTTP API, embedded UI, CLI. |
| `web` | SolidJS + Vite single-page app: pages, components, ECharts builders, bilingual dictionaries, design tokens. Built into `web/dist` and embedded in the binary at compile time. |
| `config` | The universe generated from honeclaw's ontology (`universe.json`) and its bilingual overlay. |
| `deploy`, `scripts`, `docs` | systemd units, PostgreSQL setup, runtime template, release/deploy/backup scripts, runbooks. |

## Domain model

- **Universe**: sectors and assets (from the honeclaw ontology), versioned by content hash in
  `universe_versions`; removed names are deactivated, never deleted.
- **Portfolios**: `portfolios` are named, isolated paper books with an owner (a user's audit
  identity, or none for a shared portfolio run by administrators) and their own automation mode.
  A portfolio owns a series of paper accounts, exactly one of them active while the portfolio is
  active; a reset archives the account and opens a new one in the same portfolio. Holdings,
  cash, plans, fills, NAV history, the active strategy version, cancelled slots and
  portfolio-only restrictions are per portfolio. The universe, the strategy library, market data,
  the schedule, costs, risk thresholds, notification channels, reminders and backtests are shared.
- **Paper account**: `accounts` (one active per portfolio; `mode` is constrained to `'paper'` by
  a database CHECK), `positions` (quantity, average cost, realised P&L, dividends), `cash_ledger`
  (every cash movement with the running balance), `nav_snapshots` and `position_snapshots`
  (end of day).
- **Strategy**: immutable `strategy_versions` (validated parameters), a library shared by every
  portfolio, and `strategy_activations` (which version each account trades, who activated it,
  when and why). `trading_restrictions` hold exclusions and locks, for every portfolio
  (`portfolio_id` NULL; these win) or for one.
- **Plans**: one row per account, trade date and slot (`open`, `close`, or any number of
  `manual`), with the targets, diagnostics and summary stored as JSON; `orders` (planned trades
  with weights before/target/after) and `fills` (executed trades with prices, costs, realised
  P&L). `skipped_slots` records a portfolio's slots cancelled in advance.
- **Operations**: `notifications` (rendered in both languages, with per-channel delivery
  results, tagged with the portfolio they are about), `reminders`, `settings` (typed JSON
  documents, validated on write), `audit_log` (append-only), `job_runs` (idempotent scheduler
  jobs), `backtests` (config, summary, result).

## Plan lifecycle

```
            generate (scheduler or operator)
                     │
                     ▼
   no_action ◀── pending ──▶ cancelled (operator, or "cancel the rest of the day")
                     │  ╲
   review window ends │   ╲ deadline passes (approval mode, not approved)
   or operator approves   ▼
                     ▼    expired
                 executing ──▶ executed | partially_executed | failed
```

A slot that should have produced a plan but did not is recorded as `skipped` with the reason
(automation paused, cancelled by an operator, missed while the service was down, or an error).

## Scheduler

One instance is the leader (a PostgreSQL session advisory lock held on a dedicated connection),
so a second instance or a restart overlap can never double-trade. It ticks every 15 seconds and
runs each job at most once per key (`job_runs` has a unique `(job, run_key)`; failed runs are
retried after ten minutes). Plan slots, executions, risk checks, snapshots and reports run for
every active portfolio, keyed per portfolio (`<date>:p<id>`):

| Job (New York time) | What it does |
| --- | --- |
| Pre-open sync, open − 90 min | daily bars, corporate actions, quotes; pre-open reminder |
| Plan slots, open + 30 min and close − 3 h | generate the plan (or record why not) |
| Execution | after the review window (automatic) or on approval; expiry at the deadline |
| Quotes | every 60 s during the session (configurable) |
| Risk checks | every 5 min: drawdown and daily-loss thresholds, stale data |
| Post-close sync, close + 15 min | final bars and closes, end-of-day snapshot, daily summary |
| Reports & reminders | weekly report, custom reminders, deferred (quiet-hours) notifications |

Everything that changes a paper account — plan generation, execution, corporate actions,
account resets, archiving a portfolio — is serialised by one in-process lock, and each plan
executes in a single database transaction after re-checking every quote.

## Time

- Market logic uses `America/New_York` (sessions, holidays, early closes, DST); display uses the
  configured zone (Singapore by default) next to New York time.
- Business timestamps use the application clock: `app_now()` in SQL equals `now()` in
  production. The demo mode can start the clock at any instant (`HONE_QUANT_DEV_CLOCK`) to
  rehearse a trading day; the database then follows the same offset. Sessions and sign-in
  always use the wall clock.

## Market data

- **FMP** client with the stable API and legacy fallback, key rotation, a shared rate limiter
  and endpoint diagnostics (`hone-quant fmp-check`). Data: batch quotes, ten years of daily
  bars (split- and dividend-adjusted closes stored alongside raw prices), 5/15-minute intraday
  bars for the charts (cached briefly), splits and dividends.
- **Demo** market: a deterministic synthetic market (regimes, sector correlation, listing
  dates) for evaluation, tests and screenshots. A schema remembers which source created it, so
  demo data can never mix with real data.

## Security

- Paper only: there is no brokerage client in the code base, and the database rejects any
  account mode other than `paper`.
- Operators sign in with Argon2id-hashed passwords; sessions are random 256-bit tokens (stored
  hashed) in `HttpOnly; SameSite=Strict` cookies, with failed-login throttling. Roles: admin
  (every portfolio and setting), member (creates and runs their own portfolios, reads what is
  shared; sees only their own portfolios, notifications and audit entries) and viewer
  (read-only).
- The web app sends the selected portfolio in `X-Hone-Quant-Portfolio` (`?portfolio=` for
  EventSource and links); the server checks visibility and the right to act on every request,
  and plans addressed by id are authorised against their own portfolio. Server-sent events
  about a portfolio reach only users who can see it.
- Mutating requests must carry the `X-Hone-Quant-Action` header (cross-site forms cannot).
  Responses carry a strict Content-Security-Policy (no inline scripts), `X-Frame-Options: DENY`,
  `nosniff` and a strict referrer policy.
- Notification-channel credentials are sealed with ChaCha20-Poly1305 using a key in the state
  directory (or `HONE_QUANT_SECRET_KEY`) and are never returned in full by the API.
- The server binds to loopback by default; remote access goes through an IAP/SSH tunnel or an
  HTTPS reverse proxy (docs/deployment-gce.md).

## Web UI

- SolidJS with lazily loaded pages; the API is the single source of truth and pages refetch
  when the server pushes a change over SSE (plans, account, quotes, notifications, settings).
- Bilingual: Chinese is the canonical dictionary and English is type-checked against it, page by
  page; every number, money amount and time is formatted for the active language, and times are
  shown in the local zone and New York.
- Design tokens follow honeclaw's visual language (light and dark themes) plus market colours
  that can be switched between green-up, red-up and a colour-blind-friendly blue/orange.
- Charts (ECharts) follow a few strict rules: one value axis per chart, legends and direct labels
  for multiple series, hollow-up/filled-down candles so direction never relies on colour alone.

## Testing

- `quant-core`: unit tests for the calendar (holidays, DST, early closes), schedule, statistics,
  allocation, strategy engine, rebalancer, costs, metrics and backtester.
- `quant-server`: unit and integration tests (stores, broker, scheduler pieces, notification
  channels with a local mock server, FMP wire formats, demo market) against a real PostgreSQL
  (`HONE_QUANT_TEST_DATABASE_URL`, one throw-away schema per test).
- `web`: unit tests (formatting, chart layout, names) with Bun, plus TypeScript in strict mode.
- CI runs all of the above on every push (`.github/workflows/ci.yml`).
