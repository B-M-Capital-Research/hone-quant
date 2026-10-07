-- hone-quant schema v2: portfolios.
--
-- A portfolio is a named, isolated paper book. It owns a series of paper accounts (a reset
-- archives the current account and opens a new one in the same portfolio), its own automation
-- mode, its own active strategy version (strategy_activations are per account) and its own
-- cancelled slots and restrictions. The universe, the strategy library, market data and every
-- other setting stay shared by all portfolios.
--
-- Existing data becomes one shared portfolio, "Main", that inherits the global automation setting.

CREATE TABLE portfolios (
    id           BIGSERIAL PRIMARY KEY,
    name         TEXT NOT NULL CHECK (length(btrim(name)) BETWEEN 1 AND 60),
    description  TEXT NOT NULL DEFAULT '',
    -- Audit identity of the owner (local username or `honeclaw:<id>`); NULL = shared, managed by
    -- administrators.
    owner        TEXT,
    owner_name   TEXT NOT NULL DEFAULT '',
    -- `AutomationSettings`: mode, paused_until, note.
    automation   JSONB NOT NULL DEFAULT '{}'::jsonb,
    status       TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'archived')),
    created_by   TEXT NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT app_now(),
    archived_at  TIMESTAMPTZ,
    archived_by  TEXT
);
-- Names are unique among an owner's active portfolios (shared ones count as one owner).
CREATE UNIQUE INDEX portfolios_name_unique ON portfolios (COALESCE(owner, ''), lower(btrim(name)))
    WHERE status = 'active';
CREATE INDEX portfolios_owner_idx ON portfolios (owner);

INSERT INTO portfolios (name, automation, created_by)
SELECT 'Main', COALESCE((SELECT value FROM settings WHERE key = 'automation'), '{}'::jsonb), 'migration'
WHERE EXISTS (SELECT 1 FROM accounts);

-- Accounts belong to a portfolio; one active account per portfolio instead of one overall.
ALTER TABLE accounts ADD COLUMN portfolio_id BIGINT REFERENCES portfolios (id);
UPDATE accounts SET portfolio_id = (SELECT min(id) FROM portfolios);
ALTER TABLE accounts ALTER COLUMN portfolio_id SET NOT NULL;
DROP INDEX accounts_single_active;
CREATE UNIQUE INDEX accounts_active_per_portfolio ON accounts (portfolio_id) WHERE status = 'active';
CREATE INDEX accounts_portfolio_idx ON accounts (portfolio_id, id DESC);

-- Cancelled slots are per portfolio.
ALTER TABLE skipped_slots ADD COLUMN portfolio_id BIGINT REFERENCES portfolios (id);
UPDATE skipped_slots SET portfolio_id = (SELECT min(id) FROM portfolios);
DELETE FROM skipped_slots WHERE portfolio_id IS NULL;
ALTER TABLE skipped_slots DROP CONSTRAINT skipped_slots_pkey;
ALTER TABLE skipped_slots ALTER COLUMN portfolio_id SET NOT NULL;
ALTER TABLE skipped_slots ADD PRIMARY KEY (portfolio_id, trade_date, slot);

-- Restrictions apply to every portfolio (NULL, as before) or to one.
ALTER TABLE trading_restrictions ADD COLUMN portfolio_id BIGINT REFERENCES portfolios (id);
CREATE INDEX trading_restrictions_portfolio_idx ON trading_restrictions (portfolio_id);

-- Notifications about one portfolio (NULL: about the whole system).
ALTER TABLE notifications ADD COLUMN portfolio_id BIGINT REFERENCES portfolios (id);
CREATE INDEX notifications_portfolio_idx ON notifications (portfolio_id, ts DESC);

-- Members manage their own portfolios; admins everything; viewers read.
ALTER TABLE users DROP CONSTRAINT users_role_check;
ALTER TABLE users ADD CONSTRAINT users_role_check CHECK (role IN ('admin', 'member', 'viewer'));

-- The automation mode now lives on each portfolio.
DELETE FROM settings WHERE key = 'automation';
