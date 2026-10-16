-- hone-quant schema v3: broker-linked accounts.
--
-- Until now every account was simulated by hone-quant's own paper broker ('sim'). An account can
-- now mirror an account held at a broker instead: Longbridge, operated through its `longbridge`
-- command-line client. Plans are then executed by submitting orders to the broker, and the
-- broker's cash, positions, orders and executions are synchronised back into the same tables.
-- `mode` says whether the account trades simulated (paper) or real money; hone-quant's own
-- simulator is always paper.

ALTER TABLE accounts ADD COLUMN broker TEXT NOT NULL DEFAULT 'sim'
    CHECK (broker IN ('sim', 'longbridge'));
ALTER TABLE accounts DROP CONSTRAINT accounts_mode_check;
ALTER TABLE accounts ADD CONSTRAINT accounts_mode_check
    CHECK (mode IN ('paper', 'live') AND (mode = 'paper' OR broker <> 'sim'));
-- The broker account this account mirrors, as the broker identifies it (Longbridge: the account
-- channel, e.g. `lb_papertrading`, and the account id when the broker reports one).
ALTER TABLE accounts ADD COLUMN broker_account TEXT;
ALTER TABLE accounts ADD CONSTRAINT accounts_broker_identity
    CHECK (broker = 'sim' OR broker_account IS NOT NULL);
-- Last successful synchronisation, and the latest attempt's report (errors, what was adjusted,
-- holdings hone-quant does not manage).
ALTER TABLE accounts ADD COLUMN synced_at TIMESTAMPTZ;
ALTER TABLE accounts ADD COLUMN sync_status JSONB NOT NULL DEFAULT '{}'::jsonb;
-- Two portfolios can never trade the same broker account.
CREATE UNIQUE INDEX accounts_broker_account_active ON accounts (broker, broker_account)
    WHERE status = 'active' AND broker <> 'sim';

-- Orders sent to a broker: 'submitting' while the request is in flight (recovered by its remark
-- if hone-quant stops before it learns the broker's order id), 'submitted' while it works at the
-- broker. The broker's own status is kept verbatim next to hone-quant's.
ALTER TABLE orders DROP CONSTRAINT orders_status_check;
ALTER TABLE orders ADD CONSTRAINT orders_status_check CHECK (status IN (
    'planned', 'submitting', 'submitted', 'skipped', 'filled', 'partially_filled', 'rejected',
    'cancelled', 'expired'));
ALTER TABLE orders ADD COLUMN broker_order_id TEXT;
ALTER TABLE orders ADD COLUMN broker_status TEXT;
-- The broker's explanation when it refused or ended the order.
ALTER TABLE orders ADD COLUMN broker_message TEXT;
-- Quantity and limit price actually sent (a buy can be reduced to the cash available), and the
-- quote the order was priced from (the reference for its fills' slippage).
ALTER TABLE orders ADD COLUMN broker_qty NUMERIC(20, 6);
ALTER TABLE orders ADD COLUMN limit_price NUMERIC(20, 6);
ALTER TABLE orders ADD COLUMN quote_price DOUBLE PRECISION;
ALTER TABLE orders ADD COLUMN quote_ts TIMESTAMPTZ;
ALTER TABLE orders ADD COLUMN submitted_at TIMESTAMPTZ;
CREATE UNIQUE INDEX orders_broker_order_unique ON orders (account_id, broker_order_id)
    WHERE broker_order_id IS NOT NULL;
CREATE INDEX orders_working_idx ON orders (account_id) WHERE status IN ('submitting', 'submitted');

-- Executions reported by the broker are recorded once, by the broker's trade id.
ALTER TABLE fills ADD COLUMN broker_trade_id TEXT;
CREATE UNIQUE INDEX fills_broker_trade_unique ON fills (account_id, broker_trade_id)
    WHERE broker_trade_id IS NOT NULL;
