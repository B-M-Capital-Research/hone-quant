-- hone-quant schema v1.
--
-- Every object lives in the dedicated schema selected by the server (search_path is pinned per
-- connection), so hone-quant can share a PostgreSQL instance — or even honeclaw's database —
-- without touching honeclaw's `public` tables. Money and quantities are NUMERIC; analytics
-- inputs (prices, weights) are DOUBLE PRECISION.

-- Application clock ----------------------------------------------------------------------------
--
-- Business timestamps (quotes, plans, fills, ledger, notifications, audit) use app_now(). It is
-- exactly now() in production. A demo server started with HONE_QUANT_DEV_CLOCK sets
-- hone_quant.clock_offset_secs on its connections so that database timestamps follow the same
-- simulated clock as the scheduler. Authentication (users, sessions) always uses the wall clock.

CREATE FUNCTION app_now() RETURNS TIMESTAMPTZ
    LANGUAGE sql STABLE PARALLEL SAFE
    AS $$
        SELECT now() + make_interval(secs => COALESCE(
            NULLIF(current_setting('hone_quant.clock_offset_secs', true), '')::DOUBLE PRECISION, 0))
    $$;

-- Instance -------------------------------------------------------------------------------------

CREATE TABLE instance_meta (
    key         TEXT PRIMARY KEY,
    value       JSONB NOT NULL,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT app_now()
);

-- Operators ------------------------------------------------------------------------------------

CREATE TABLE users (
    id                  BIGSERIAL PRIMARY KEY,
    username            TEXT NOT NULL UNIQUE,
    password_hash       TEXT NOT NULL,
    role                TEXT NOT NULL CHECK (role IN ('admin', 'viewer')),
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    password_changed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_login_at       TIMESTAMPTZ
);

CREATE TABLE sessions (
    token_hash    TEXT PRIMARY KEY,
    user_id       BIGINT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at    TIMESTAMPTZ NOT NULL,
    last_seen_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    user_agent    TEXT NOT NULL DEFAULT '',
    ip            TEXT NOT NULL DEFAULT ''
);
CREATE INDEX sessions_user_idx ON sessions (user_id);
CREATE INDEX sessions_expiry_idx ON sessions (expires_at);

-- Universe (from the honeclaw ontology) --------------------------------------------------------

CREATE TABLE sectors (
    id           TEXT PRIMARY KEY,
    name_zh      TEXT NOT NULL,
    name_en      TEXT NOT NULL,
    summary_zh   TEXT NOT NULL DEFAULT '',
    summary_en   TEXT NOT NULL DEFAULT '',
    sort_order   INT NOT NULL DEFAULT 0,
    is_active    BOOLEAN NOT NULL DEFAULT true,
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT app_now()
);

CREATE TABLE assets (
    symbol       TEXT PRIMARY KEY,
    name_zh      TEXT NOT NULL,
    name_en      TEXT NOT NULL,
    -- Primary sector: the one the asset is budgeted in.
    sector_id    TEXT NOT NULL REFERENCES sectors (id),
    subtype_id   TEXT NOT NULL DEFAULT '',
    subtype_zh   TEXT NOT NULL DEFAULT '',
    subtype_en   TEXT NOT NULL DEFAULT '',
    -- Other ontology sectors the company also appears in (context only).
    also_in      TEXT[] NOT NULL DEFAULT '{}',
    -- Ontology `role`: research context, explicitly not a rating.
    role_zh      TEXT NOT NULL DEFAULT '',
    sort_order   INT NOT NULL DEFAULT 0,
    is_active    BOOLEAN NOT NULL DEFAULT true,
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT app_now()
);
CREATE INDEX assets_sector_idx ON assets (sector_id);

CREATE TABLE universe_versions (
    id                       BIGSERIAL PRIMARY KEY,
    source                   TEXT NOT NULL,
    ontology_schema_version  INT,
    ontology_generated_at    TEXT,
    content_hash             TEXT NOT NULL,
    changes                  JSONB NOT NULL,
    applied_by               TEXT NOT NULL,
    applied_at               TIMESTAMPTZ NOT NULL DEFAULT app_now()
);

-- Market data ----------------------------------------------------------------------------------

CREATE TABLE daily_bars (
    symbol      TEXT NOT NULL,
    date        DATE NOT NULL,
    open        DOUBLE PRECISION NOT NULL,
    high        DOUBLE PRECISION NOT NULL,
    low         DOUBLE PRECISION NOT NULL,
    close       DOUBLE PRECISION NOT NULL,
    volume      DOUBLE PRECISION NOT NULL DEFAULT 0,
    -- Split- and dividend-adjusted (total return); NULL when the provider plan lacks them.
    adj_open    DOUBLE PRECISION,
    adj_close   DOUBLE PRECISION,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT app_now(),
    PRIMARY KEY (symbol, date)
);
CREATE INDEX daily_bars_date_idx ON daily_bars (date);

CREATE TABLE intraday_bars (
    symbol      TEXT NOT NULL,
    interval    TEXT NOT NULL,
    ts          TIMESTAMPTZ NOT NULL,
    open        DOUBLE PRECISION NOT NULL,
    high        DOUBLE PRECISION NOT NULL,
    low         DOUBLE PRECISION NOT NULL,
    close       DOUBLE PRECISION NOT NULL,
    volume      DOUBLE PRECISION NOT NULL DEFAULT 0,
    fetched_at  TIMESTAMPTZ NOT NULL DEFAULT app_now(),
    PRIMARY KEY (symbol, interval, ts)
);

CREATE TABLE quotes (
    symbol      TEXT PRIMARY KEY,
    price       DOUBLE PRECISION NOT NULL,
    change      DOUBLE PRECISION,
    change_pct  DOUBLE PRECISION,
    open        DOUBLE PRECISION,
    day_high    DOUBLE PRECISION,
    day_low     DOUBLE PRECISION,
    prev_close  DOUBLE PRECISION,
    volume      DOUBLE PRECISION,
    avg_volume  DOUBLE PRECISION,
    market_cap  DOUBLE PRECISION,
    quote_ts    TIMESTAMPTZ,
    fetched_at  TIMESTAMPTZ NOT NULL DEFAULT app_now()
);

CREATE TABLE corporate_actions (
    id          BIGSERIAL PRIMARY KEY,
    symbol      TEXT NOT NULL,
    kind        TEXT NOT NULL CHECK (kind IN ('split', 'dividend')),
    ex_date     DATE NOT NULL,
    pay_date    DATE,
    -- Splits: shares after / shares before. Dividends: cash per share.
    ratio       DOUBLE PRECISION,
    amount      DOUBLE PRECISION,
    fetched_at  TIMESTAMPTZ NOT NULL DEFAULT app_now(),
    UNIQUE (symbol, kind, ex_date)
);

-- Paper account --------------------------------------------------------------------------------

CREATE TABLE accounts (
    id              BIGSERIAL PRIMARY KEY,
    name            TEXT NOT NULL,
    base_currency   TEXT NOT NULL DEFAULT 'USD',
    -- Always 'paper': hone-quant has no live-broker integration.
    mode            TEXT NOT NULL DEFAULT 'paper' CHECK (mode = 'paper'),
    initial_cash    NUMERIC(20, 2) NOT NULL CHECK (initial_cash > 0),
    cash            NUMERIC(20, 2) NOT NULL,
    inception_date  DATE NOT NULL,
    status          TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'archived')),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT app_now(),
    archived_at     TIMESTAMPTZ,
    CONSTRAINT accounts_cash_non_negative CHECK (cash >= 0)
);
CREATE UNIQUE INDEX accounts_single_active ON accounts (status) WHERE status = 'active';

CREATE TABLE positions (
    account_id    BIGINT NOT NULL REFERENCES accounts (id),
    symbol        TEXT NOT NULL,
    qty           NUMERIC(20, 6) NOT NULL CHECK (qty >= 0),
    avg_cost      NUMERIC(20, 6) NOT NULL DEFAULT 0,
    realized_pnl  NUMERIC(20, 2) NOT NULL DEFAULT 0,
    dividends     NUMERIC(20, 2) NOT NULL DEFAULT 0,
    opened_at     TIMESTAMPTZ,
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT app_now(),
    PRIMARY KEY (account_id, symbol)
);

CREATE TABLE cash_ledger (
    id             BIGSERIAL PRIMARY KEY,
    account_id     BIGINT NOT NULL REFERENCES accounts (id),
    ts             TIMESTAMPTZ NOT NULL DEFAULT app_now(),
    kind           TEXT NOT NULL CHECK (kind IN ('deposit', 'trade', 'dividend', 'adjustment')),
    amount         NUMERIC(20, 2) NOT NULL,
    balance_after  NUMERIC(20, 2) NOT NULL,
    symbol         TEXT,
    ref_type       TEXT,
    ref_id         TEXT,
    note           TEXT NOT NULL DEFAULT ''
);
CREATE INDEX cash_ledger_account_idx ON cash_ledger (account_id, ts DESC);

CREATE TABLE corporate_action_applications (
    id            BIGSERIAL PRIMARY KEY,
    account_id    BIGINT NOT NULL REFERENCES accounts (id),
    action_id     BIGINT NOT NULL REFERENCES corporate_actions (id),
    qty_before    NUMERIC(20, 6) NOT NULL,
    qty_after     NUMERIC(20, 6) NOT NULL,
    cash_amount   NUMERIC(20, 2) NOT NULL DEFAULT 0,
    applied_at    TIMESTAMPTZ NOT NULL DEFAULT app_now(),
    UNIQUE (account_id, action_id)
);

-- Strategy -------------------------------------------------------------------------------------

-- Immutable, append-only configurations. Editing a strategy creates a new version.
CREATE TABLE strategy_versions (
    id          BIGSERIAL PRIMARY KEY,
    name        TEXT NOT NULL,
    preset_id   TEXT NOT NULL,
    params      JSONB NOT NULL,
    note        TEXT NOT NULL DEFAULT '',
    created_by  TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT app_now()
);

-- Which version was active for an account, and since when. The latest row is the active one.
CREATE TABLE strategy_activations (
    id                   BIGSERIAL PRIMARY KEY,
    account_id           BIGINT NOT NULL REFERENCES accounts (id),
    strategy_version_id  BIGINT NOT NULL REFERENCES strategy_versions (id),
    activated_by         TEXT NOT NULL,
    note                 TEXT NOT NULL DEFAULT '',
    activated_at         TIMESTAMPTZ NOT NULL DEFAULT app_now()
);
CREATE INDEX strategy_activations_account_idx ON strategy_activations (account_id, activated_at DESC);

-- Operator controls ----------------------------------------------------------------------------

-- 'exclude' = target weight zero (positions are sold); 'lock' = hold the position, never trade.
CREATE TABLE trading_restrictions (
    id          BIGSERIAL PRIMARY KEY,
    symbol      TEXT NOT NULL,
    mode        TEXT NOT NULL CHECK (mode IN ('exclude', 'lock')),
    reason      TEXT NOT NULL DEFAULT '',
    starts_on   DATE NOT NULL,
    ends_on     DATE,
    created_by  TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT app_now(),
    revoked_at  TIMESTAMPTZ,
    revoked_by  TEXT
);
CREATE INDEX trading_restrictions_symbol_idx ON trading_restrictions (symbol);

-- Plan slots the operator cancelled in advance ("cancel today's planned trades").
CREATE TABLE skipped_slots (
    trade_date  DATE NOT NULL,
    slot        TEXT NOT NULL CHECK (slot IN ('open', 'close')),
    reason      TEXT NOT NULL DEFAULT '',
    created_by  TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT app_now(),
    PRIMARY KEY (trade_date, slot)
);

-- Plans, orders, fills -------------------------------------------------------------------------

CREATE TABLE plans (
    id                   BIGSERIAL PRIMARY KEY,
    account_id           BIGINT NOT NULL REFERENCES accounts (id),
    trade_date           DATE NOT NULL,
    slot                 TEXT NOT NULL CHECK (slot IN ('open', 'close', 'manual')),
    status               TEXT NOT NULL CHECK (status IN (
                             'pending', 'executing', 'executed', 'partially_executed',
                             'no_action', 'cancelled', 'expired', 'skipped', 'failed')),
    strategy_version_id  BIGINT REFERENCES strategy_versions (id),
    automation_mode      TEXT NOT NULL,
    generated_at         TIMESTAMPTZ NOT NULL DEFAULT app_now(),
    -- Automatic execution time; NULL when the plan waits for approval.
    execute_after        TIMESTAMPTZ,
    deadline             TIMESTAMPTZ NOT NULL,
    approved_at          TIMESTAMPTZ,
    approved_by          TEXT,
    cancelled_at         TIMESTAMPTZ,
    cancelled_by         TEXT,
    cancel_reason        TEXT,
    executed_at          TIMESTAMPTZ,
    nav                  NUMERIC(20, 2) NOT NULL DEFAULT 0,
    cash                 NUMERIC(20, 2) NOT NULL DEFAULT 0,
    exposure_target      DOUBLE PRECISION,
    invested_target      DOUBLE PRECISION,
    breadth              DOUBLE PRECISION,
    est_vol              DOUBLE PRECISION,
    turnover             DOUBLE PRECISION NOT NULL DEFAULT 0,
    est_costs            NUMERIC(20, 2) NOT NULL DEFAULT 0,
    order_count          INT NOT NULL DEFAULT 0,
    -- Full engine output, rebalance decisions and the quote snapshot the plan was built from.
    diagnostics          JSONB NOT NULL DEFAULT '{}'::jsonb,
    summary              JSONB NOT NULL DEFAULT '{}'::jsonb,
    error                TEXT,
    created_by           TEXT NOT NULL DEFAULT 'system'
);
CREATE UNIQUE INDEX plans_slot_unique ON plans (account_id, trade_date, slot) WHERE slot <> 'manual';
CREATE INDEX plans_date_idx ON plans (account_id, trade_date DESC, generated_at DESC);
CREATE INDEX plans_status_idx ON plans (status);

CREATE TABLE orders (
    id             BIGSERIAL PRIMARY KEY,
    plan_id        BIGINT NOT NULL REFERENCES plans (id),
    account_id     BIGINT NOT NULL REFERENCES accounts (id),
    symbol         TEXT NOT NULL,
    side           TEXT NOT NULL CHECK (side IN ('buy', 'sell')),
    reason         TEXT NOT NULL CHECK (reason IN ('entry', 'exit', 'increase', 'decrease')),
    qty            NUMERIC(20, 6) NOT NULL CHECK (qty > 0),
    ref_price      DOUBLE PRECISION NOT NULL,
    weight_before  DOUBLE PRECISION NOT NULL,
    weight_target  DOUBLE PRECISION NOT NULL,
    weight_after   DOUBLE PRECISION NOT NULL,
    status         TEXT NOT NULL CHECK (status IN (
                       'planned', 'skipped', 'filled', 'partially_filled', 'rejected',
                       'cancelled', 'expired')),
    status_reason  TEXT,
    filled_qty     NUMERIC(20, 6) NOT NULL DEFAULT 0,
    sequence       INT NOT NULL DEFAULT 0,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT app_now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT app_now()
);
CREATE INDEX orders_plan_idx ON orders (plan_id, sequence);
CREATE INDEX orders_symbol_idx ON orders (account_id, symbol, created_at DESC);

CREATE TABLE fills (
    id            BIGSERIAL PRIMARY KEY,
    order_id      BIGINT NOT NULL REFERENCES orders (id),
    account_id    BIGINT NOT NULL REFERENCES accounts (id),
    symbol        TEXT NOT NULL,
    side          TEXT NOT NULL CHECK (side IN ('buy', 'sell')),
    qty           NUMERIC(20, 6) NOT NULL CHECK (qty > 0),
    price         NUMERIC(20, 6) NOT NULL,
    quote_price   DOUBLE PRECISION NOT NULL,
    quote_ts      TIMESTAMPTZ,
    notional      NUMERIC(20, 2) NOT NULL,
    commission    NUMERIC(20, 2) NOT NULL,
    fees          NUMERIC(20, 2) NOT NULL,
    slippage      NUMERIC(20, 2) NOT NULL,
    realized_pnl  NUMERIC(20, 2),
    executed_at   TIMESTAMPTZ NOT NULL DEFAULT app_now()
);
CREATE INDEX fills_account_idx ON fills (account_id, executed_at DESC);
CREATE INDEX fills_symbol_idx ON fills (account_id, symbol, executed_at DESC);

-- Valuation history ----------------------------------------------------------------------------

CREATE TABLE nav_snapshots (
    account_id  BIGINT NOT NULL REFERENCES accounts (id),
    date        DATE NOT NULL,
    nav         NUMERIC(20, 2) NOT NULL,
    cash        NUMERIC(20, 2) NOT NULL,
    invested    NUMERIC(20, 2) NOT NULL,
    flows       NUMERIC(20, 2) NOT NULL DEFAULT 0,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT app_now(),
    PRIMARY KEY (account_id, date)
);

CREATE TABLE position_snapshots (
    account_id  BIGINT NOT NULL REFERENCES accounts (id),
    date        DATE NOT NULL,
    symbol      TEXT NOT NULL,
    qty         NUMERIC(20, 6) NOT NULL,
    price       DOUBLE PRECISION NOT NULL,
    value       NUMERIC(20, 2) NOT NULL,
    weight      DOUBLE PRECISION NOT NULL,
    PRIMARY KEY (account_id, date, symbol)
);

-- Research -------------------------------------------------------------------------------------

CREATE TABLE backtests (
    id                   BIGSERIAL PRIMARY KEY,
    name                 TEXT NOT NULL,
    status               TEXT NOT NULL CHECK (status IN ('queued', 'running', 'succeeded', 'failed')),
    config               JSONB NOT NULL,
    strategy_version_id  BIGINT REFERENCES strategy_versions (id),
    summary              JSONB,
    result               JSONB,
    error                TEXT,
    created_by           TEXT NOT NULL,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT app_now(),
    started_at           TIMESTAMPTZ,
    finished_at          TIMESTAMPTZ
);
CREATE INDEX backtests_created_idx ON backtests (created_at DESC);

-- Notifications & reminders --------------------------------------------------------------------

CREATE TABLE notifications (
    id           BIGSERIAL PRIMARY KEY,
    ts           TIMESTAMPTZ NOT NULL DEFAULT app_now(),
    kind         TEXT NOT NULL,
    category     TEXT NOT NULL,
    severity     TEXT NOT NULL CHECK (severity IN ('info', 'success', 'warning', 'critical')),
    title_zh     TEXT NOT NULL,
    title_en     TEXT NOT NULL,
    body_zh      TEXT NOT NULL DEFAULT '',
    body_en      TEXT NOT NULL DEFAULT '',
    params       JSONB NOT NULL DEFAULT '{}'::jsonb,
    link         TEXT,
    read_at      TIMESTAMPTZ,
    -- Delivery attempts per outbound channel.
    deliveries   JSONB NOT NULL DEFAULT '[]'::jsonb,
    -- Outbound delivery deferred by quiet hours.
    deferred     BOOLEAN NOT NULL DEFAULT false
);
CREATE INDEX notifications_ts_idx ON notifications (ts DESC);
CREATE INDEX notifications_unread_idx ON notifications (ts DESC) WHERE read_at IS NULL;
CREATE INDEX notifications_deferred_idx ON notifications (ts) WHERE deferred;

CREATE TABLE reminders (
    id             BIGSERIAL PRIMARY KEY,
    kind           TEXT NOT NULL,
    title          TEXT NOT NULL DEFAULT '',
    note           TEXT NOT NULL DEFAULT '',
    schedule       JSONB NOT NULL,
    enabled        BOOLEAN NOT NULL DEFAULT true,
    last_fired_at  TIMESTAMPTZ,
    next_fire_at   TIMESTAMPTZ,
    created_by     TEXT NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT app_now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT app_now()
);
CREATE UNIQUE INDEX reminders_builtin_unique ON reminders (kind) WHERE kind <> 'custom';

-- Settings, audit, jobs ------------------------------------------------------------------------

CREATE TABLE settings (
    key         TEXT PRIMARY KEY,
    value       JSONB NOT NULL,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT app_now(),
    updated_by  TEXT NOT NULL DEFAULT 'system'
);

CREATE TABLE audit_log (
    id           BIGSERIAL PRIMARY KEY,
    ts           TIMESTAMPTZ NOT NULL DEFAULT app_now(),
    actor        TEXT NOT NULL,
    action       TEXT NOT NULL,
    entity_type  TEXT NOT NULL DEFAULT '',
    entity_id    TEXT NOT NULL DEFAULT '',
    detail       JSONB NOT NULL DEFAULT '{}'::jsonb,
    ip           TEXT NOT NULL DEFAULT ''
);
CREATE INDEX audit_log_ts_idx ON audit_log (ts DESC);
CREATE INDEX audit_log_entity_idx ON audit_log (entity_type, entity_id);

CREATE TABLE job_runs (
    id           BIGSERIAL PRIMARY KEY,
    job          TEXT NOT NULL,
    run_key      TEXT NOT NULL,
    status       TEXT NOT NULL CHECK (status IN ('running', 'succeeded', 'failed', 'skipped')),
    started_at   TIMESTAMPTZ NOT NULL DEFAULT app_now(),
    finished_at  TIMESTAMPTZ,
    detail       JSONB NOT NULL DEFAULT '{}'::jsonb,
    error        TEXT,
    UNIQUE (job, run_key)
);
CREATE INDEX job_runs_started_idx ON job_runs (started_at DESC);
