# Trading through a Longbridge paper account

[中文](longbridge.zh.md)

A portfolio can trade on hone-quant's own simulator (the default) or be **linked to a Longbridge
paper-trading account**. A linked portfolio runs the same strategy, schedule, review windows and
operator controls, but its orders go to Longbridge and its books follow the broker: every fill,
the cash and the holdings come from Longbridge's reports. hone-quant drives Longbridge through
the open-source [`longbridge` command-line client](https://github.com/longbridge/longbridge-terminal)
(longbridge-terminal 0.30 or later).

Only **paper** accounts can be linked. The design is ready for live accounts (see
[Live trading](#live-trading-not-enabled)), but nothing in this version enables them.

## How it works

```
 scheduler ─▶ planner ─▶ plan (review window, approval) ─▶ execution
                 ▲                                            │
                 │ cash, holdings                     orders  ▼
          synchronisation ◀──── longbridge serve (JSON-RPC over stdio) ────▶ Longbridge paper account
                 ▲                                            │
                 └──────────── fills, order states ◀──────────┘
```

- **Transport.** hone-quant runs `longbridge serve`, the client's interface for other software:
  JSON-RPC 2.0 over standard input and output carrying Longbridge OpenAPI's raw payloads (exact
  decimals, documented shapes). The client's `--format json` output is reshaped for people and
  AI agents and may change between releases, so it is not used for trading. Two one-shot
  commands complete the picture: `longbridge --version` and `longbridge auth status` (which
  account is signed in). Parsing lives in `crates/quant-server/src/brokers/longbridge/wire.rs`;
  the payloads it is tested against are in `crates/quant-server/tests/fixtures/longbridge/`.
- **Sign-in.** The client keeps its own OAuth token, encrypted and bound to the machine, under
  `$HOME/.longbridge`. hone-quant never sees Longbridge credentials. The server signs in once
  (see [Setup](#setup)); the token refreshes itself.
- **Linking.** Creating a portfolio with *Execution: Longbridge paper account* (administrators
  only) reads the account: its US-dollar cash, and its US holdings of universe companies, which
  are adopted at the broker's average cost. The starting capital is the cash plus the value of
  those holdings. Holdings in other markets, or of companies outside the universe, are reported
  as *unmanaged* and never traded. One Longbridge account can be linked to one active portfolio.
- **Planning.** Before a plan is generated, the account is synchronised, so the plan is sized on
  the broker's actual cash and holdings.
- **Execution.** When a plan executes (automatically after the review window, or on approval):
  1. the account is synchronised and checked (the same Longbridge account, still a paper one);
  2. the same quote checks as the simulator apply (fresh quote, price within the plan's
     tolerance);
  3. **sells go first**; buys follow once every sell has ended (or after ten minutes), sized to
     the US-dollar cash Longbridge reports as available;
  4. each order is a **day limit order for whole shares, regular session only**, priced 1%
     through the current quote but never beyond the plan's price tolerance
     (`max_price_deviation`, Settings → Execution). It trades at the market, while a sudden move
     cannot fill it far from the plan. Fractional quantities from the strategy are rounded down.
  5. Every order carries the remark `hq-<order id>`, visible in Longbridge's apps.
- **Following orders.** Every scheduler tick (15 s) the broker's orders and executions are read
  back. Each execution becomes one fill, booked once by the broker's trade id, moving the
  position and cash exactly like a simulated fill. When an order ends, the broker's commission
  and fees are taken from the order's charge detail and booked on its last fill. Orders still
  working at the plan's deadline (close − 5 min) are cancelled. When every order has ended the
  plan is finalised (`executed`, `partially_executed` or `failed`), synchronised and announced.
- **Synchronisation.** The broker is the source of truth. Before the open, every five minutes
  during the session, before each plan, after each plan, before the closing snapshot and on
  request (Settings → Account → *Synchronise now*), the account is compared with Longbridge.
  Any difference (a dividend, a fee, a transfer, a trade made in Longbridge's app) is booked as
  an adjustment and written to the audit log (`account.synchronised`). While orders are working
  the comparison waits, so a fill that has not been read back yet is never mistaken for a
  difference. Cash is the US-dollar balance's available plus frozen amounts (as the client's own
  portfolio view counts it).
- **Operator controls.** Approve, cancel and remove orders work as before. A plan that is
  executing at the broker can be **stopped** from its page: working orders are cancelled at
  Longbridge, unsent orders are not sent, and the plan ends with whatever filled. A linked
  portfolio cannot be reset (its books are the broker's); archive it instead.
- **Problems.** If Longbridge cannot be reached, the client is not signed in, or it is signed in
  to a different (or a live) account, nothing is traded: plan generation and execution fail
  with the reason, the account shows the error under Settings → Account, and the portfolio is
  notified (at most hourly). An order whose submission outcome is unknown (a timeout) is looked
  up by its remark and, failing that, by its symbol, side, quantity and price; it is never sent
  twice.

## Safety

- **Paper only, enforced by Longbridge.** The client runs with `LONGBRIDGE_PAPERTRADING=true`.
  The Longbridge SDK then marks every trading request as paper trading, and Longbridge's servers
  refuse it if the signed-in account is a real-money one. Independently, hone-quant checks the
  account channel (`lb_papertrading`) reported by `auth status` and with the positions, and the
  database refuses live accounts for the simulator.
- **Two-step orders.** The client never places or cancels an order on the first request: it
  answers with a dry run and a confirmation code. hone-quant checks that the dry run describes
  exactly the intended order (symbol, side, quantity, price, order type) before sending the code.
- **The right account.** The account a portfolio is linked to is recorded; if the client is later
  signed in to another account, synchronisation and trading stop with an error instead of
  adopting the other account's books.
- **Pacing and isolation.** Calls are serialised (concurrent token refreshes would race) and
  spaced a second apart, inside Longbridge's trade limit of 30 calls per 30 seconds. The client
  runs in an empty working directory (its SDK reads a `.env` from there) with analytics off
  (`DO_NOT_TRACK=1`), and an idle `serve` session is stopped after ten minutes.

## Setup

1. **Install the client** on the host that runs hone-quant. Pin a release rather than piping an
   install script, e.g. for Linux x86-64:

   ```bash
   curl -fsSL -o /tmp/lb.tgz \
     https://github.com/longbridge/longbridge-terminal/releases/download/v0.30.0/longbridge-terminal-linux-musl-amd64.tar.gz
   sudo tar -xzf /tmp/lb.tgz -C /usr/local/bin longbridge
   longbridge --version
   ```

   If it is not on the service's `PATH`, set `HONE_QUANT_LONGBRIDGE_BIN=/path/to/longbridge`.

2. **Enable a paper account** in the Longbridge developer portal (open.longbridge.com). Paper
   accounts are offered in Longbridge's AP data centre (Hong Kong / Singapore accounts). Give the
   account US-dollar cash.

3. **Sign the server in**, as the service user and with the service's environment, so the token
   lands where the server will look for it (the token is bound to the machine, so do this on the
   server itself):

   ```bash
   sudo /opt/hone-quant/current/scripts/hq.sh longbridge auth login   # device flow: open the URL, approve
   sudo /opt/hone-quant/current/scripts/hq.sh longbridge               # check: client, sign-in, account, cash, holdings
   ```

   `hone-quant longbridge <args>` runs the client exactly as the server does (same home
   directory, paper-trading guard on). On the authorisation page choose the **paper** account.
   Locally: `cargo run -p quant-server -- longbridge auth login`.

4. **Link a portfolio**: Portfolios → New portfolio → Execution: *Longbridge paper account*. The
   dialog shows whether the connection is ready and which account it reaches. Settings → Account
   then shows the broker panel: last synchronisation, cash breakdown, adjustments, unmanaged
   holdings and *Synchronise now*.

| Variable | Default | Purpose |
| --- | --- | --- |
| `HONE_QUANT_LONGBRIDGE_BIN` | `longbridge` on `PATH` | The client executable |
| `HONE_QUANT_LONGBRIDGE_HOME` | the service's `HOME` | `HOME` for the client, i.e. where its sign-in lives (`$HOME/.longbridge`) |

On the GCE install the service user's home is `/var/lib/hone-quant`, which the systemd unit can
write, so neither variable is needed when the client is in `/usr/local/bin`. The Docker image
does not include the client.

## Limits

- US equities in US dollars, regular session, day limit orders, whole shares.
- Paper accounts in Longbridge's AP data centre. US-data-centre accounts use different endpoints
  that `longbridge serve` does not route to; they are refused with a clear message.
- Longbridge does not push order updates through the client, so fills are read back by polling
  (every 15 s while orders work).
- Dividends, fees and any activity outside hone-quant reach the books as synchronisation
  adjustments, not as individual ledger lines per event.
- Performance is measured from the link date; the starting NAV is the adopted cash and holdings.

## Live trading (not enabled)

The integration is built so that live trading is a policy change rather than a rewrite:
accounts carry a `mode` (`paper`/`live`), the database allows `live` only for broker accounts,
broker clients are registered per broker *and* mode, and the identity checks work both ways (a
live client must see a non-paper channel). What is deliberately missing: no live client is
registered (`main.rs` registers the paper client only), linking refuses `mode: live`, and the UI
offers paper only. Enabling it would need, at least, a separate `HOME` signed in to the live
account, a live client without the paper guard, an explicit operator opt-in, additional
pre-trade limits (maximum order value, daily turnover) and an end-to-end review.
