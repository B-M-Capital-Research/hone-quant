# Strategy and trading methodology

中文版：[methodology.zh.md](methodology.zh.md)

hone-quant does not pick stocks. The universe — about ten AI-infrastructure sectors with a few
companies each — comes from honeclaw's industry ontology (`skills/industry-map`), and the app's
only job is to decide **how much** of each company to hold, from prices alone, and to get there
with as little trading as possible. This document describes exactly what the engine does. The
code is in `crates/quant-core` (pure functions with unit tests); the same code drives live paper
trading and backtests.

## 1. Universe

- Built from honeclaw's `industry-map.json` (schema 4) plus its live edit log, so membership changes
  made in honeclaw are replayed (`hone-quant universe build`). The bundled snapshot has 10 sectors
  and 64 companies; companies listed in two sectors (e.g. AVGO, MRVL) are held once, in their
  primary sector.
- Membership is not a recommendation and a company's ontology role is not a rating — honeclaw's
  own invariants, kept here. The universe page shows the ontology source, and updates are applied
  only after an operator reviews the diff. A removed company is sold by the next plan.
- Operators can **exclude** a company (target 0, so it is sold) or **lock** it (its position is
  frozen: no buys, no sells), with a reason and an optional end date. Both are audited.

## 2. From prices to target weights

Every plan computes target weights with the data available at that moment: daily closes up to the
previous session plus the current quote as today's price. Signals use split- and
dividend-adjusted closes.

**Eligibility.** A company needs a current price and at least `min_history_days` (126 sessions,
about six months) of history; otherwise it is reported as *no price* / *short history* and gets no
new allocation (an existing position without a price is held as is).

**Step 1 — how much to invest (market state).** *Breadth* is the share of eligible companies
trading above their 200-day average. Target exposure moves linearly between the minimum and
maximum exposure (60% and 98% by default):

> exposure = min + (max − min) × breadth

Broad weakness therefore raises cash automatically; broad strength puts it to work. Cash is the
barbell's safe end.

**Step 2 — sector budgets first.** Each sector gets a score:

- *risk* (default): the inverse of the volatility of the sector's equal-weight index over 63
  sessions (raised to `vol_power`, 1 by default) — calmer sectors get more room;
- *member count*: proportional to the number of eligible companies;
- *custom*: fixed budgets set by the operator.

The score is tilted by the sector's momentum rank (index return over 126 sessions, skipping the
most recent 21 to avoid short-term reversal): with tilt strength *s* and percentile rank *p*, the
multiplier is 1 + s × (2p − 1), i.e. between 1 − s and 1 + s (s = 0.3 by default). With
`trend_aware`, it is also multiplied by the sector's average trend multiplier (1 for members
above their 200-day average, the trend penalty for members below it), so a sector that is
breaking down cedes budget. Budgets are then allocated proportionally to the scores within a floor (3%) and a cap
(22%) per sector, and never more than the sector's members can hold under the single-name cap.

**Step 3 — companies within a sector.** Within its budget, each company is weighted by inverse
63-session volatility, tilted by its momentum rank across the whole universe (strength 0.4), and
multiplied by the *trend penalty* (0.5 by default) when its price is below its 200-day average —
faded in linearly over the first 10% below the average (`trend_ramp`), so a company hovering
around its average is not flipped between full and penalised weight at every plan.
No company may exceed the **single-name cap of 5%**; any excess is redistributed to the other
members (water-filling). Weights below the 0.5% minimum are dropped and the budget reallocated —
for a company already held the threshold is half the minimum, so a name hovering around it is
not sold and bought back from one plan to the next.

**Step 4 — optional volatility ceiling.** If a target volatility is set (off by default; 20% in
the defensive preset), the ex-ante portfolio volatility is estimated from the 63-session covariance
of the held names, and the freely allocated weights are scaled down until it fits.

Frozen (locked) names keep their current weight and are left outside these steps. Every plan
stores the full diagnostics — breadth, sector scores, budgets and caps, each company's volatility,
momentum, rank, multipliers and final weight — and shows them on the plan's page.

## 3. From targets to orders

1. Locked names and names without a usable quote are never traded.
2. Exits (target zero while holding) always trade, in full.
3. Otherwise a company trades only when the drift from target exceeds the **tolerance band**:
   the larger of 0.25 percentage points and 25% of the target weight. When it trades, it goes all
   the way to target. When the portfolio as a whole is more than 5 percentage points away from its
   target invested weight (for example while it is still being built), the bands are waived in
   the direction that closes the gap, so small shortfalls cannot stay open.
4. **Turnover cap:** if a plan's one-way turnover (Σ|trade value| / 2 / NAV) would exceed 30%, all
   non-exit trades are scaled down uniformly. A new, all-cash account therefore builds its
   portfolio over two plans (about 60% of NAV in the first, the rest in the second) instead of
   in one burst, and a strategy change is phased in gradually.
5. Whole shares (buys rounded down); trades under US$1,000 are dropped (exits excepted).
6. Sells first; buys are scaled down if cash after costs would not cover them — the paper account
   never borrows or shorts.

## 4. Schedule and automation

Two plans per trading day (NYSE calendar with holidays, early closes and special closures):

| Plan | Generated (default) | Window |
| --- | --- | --- |
| Opening plan | open + 30 min (10:00 ET) | first three hours after the open |
| Pre-close plan | close − 3 h (13:00 ET) | the last three hours before the close |

Both offsets are configurable within their windows; on early-close days only the opening plan
runs. Each plan has a **review window** (10 minutes by default) before it executes:

- **Automatic**: the plan executes when the review window ends unless someone cancels it.
- **Approval**: the plan waits for an operator to approve it and expires at its deadline
  (5 minutes before the close) otherwise.
- **Paused**: no plans are generated (optionally until a given time).

A plan is only ever generated inside its window: if the service is down until the opening
window has passed (12:30 ET), that slot is recorded as missed and notified — it is never traded
late, right before the pre-close plan.

Operators can cancel a pending plan, remove individual orders, skip an upcoming slot in advance
("cancel the rest of the day"), and restore a skipped slot. Everything is audited and notified.

## 5. Paper execution

hone-quant has no brokerage integration. Execution is simulated against the latest quote:

- Each order re-checks its quote at execution time: it must be fresher than 5 minutes and within
  3% of the price the plan was built on; otherwise the order is not filled (and says why).
  Generation also holds back companies whose quote moved more than 40% in a day (likely bad data).
- Fill price = quote ± slippage (5 bps, against the trader).
- Costs: commission US$0.005 per share (min US$1, max 1% of value), SEC fee 0.00278% of sell
  value. All costs are configurable.
- Cash, positions (average cost) and realised P&L are updated in one database transaction per
  plan; dividends are credited on the ex-date and splits adjust share counts and average cost.

## 6. Risk monitoring

- Alerts when the drawdown from the account's peak exceeds 10%, or a day's loss exceeds 3%
  (configurable), plus stale-data and failed-sync alerts.
- Notifications go to the in-app inbox and any configured channel (Telegram, Feishu, WeCom,
  Slack, Discord, signed webhook, e-mail) in Chinese or English, with quiet hours (critical
  alerts can bypass them) and reminders (pre-open, after the close, weekly review, approval
  deadlines, custom).

## 7. Backtests

The backtester runs the **same** strategy, rebalancer and cost model over daily bars:

- Opening plan → decided with closes up to the previous session plus today's open, filled at
  today's open. Pre-close plan → decided with today's close as the current price, filled at the
  close (a market-on-close idealisation of a 13:00 decision; immaterial for these slow signals).
  No signal ever sees data from after the decision.
- Prices are split- and dividend-adjusted, so returns are total returns.
- Rebalancing can run every session, weekly or monthly, with either or both slots.
- Benchmarks: SPY, QQQ, SMH and the universe as an equal-weight index (reset to equal weights
  at the start of each month, no costs; a daily-reset index would harvest a volatility premium
  that no investable strategy earns).
- Reports: total return, CAGR, volatility, Sharpe, Sortino, maximum drawdown (with dates),
  Calmar, hit rate, VaR/CVaR, beta, alpha, tracking error, information ratio, turnover and costs;
  equity and drawdown curves, monthly and yearly returns, sector weights over time, and
  contributions by sector and company.

**Read every backtest with these caveats in mind** (the report repeats them):

- *Survivorship and selection bias.* The universe is today's list of AI-infrastructure companies,
  chosen with hindsight. Past results of this universe overstate what was knowable at the time.
- *Late listings.* Companies enter only once they have enough history; the report lists them.
- *Execution.* Fills at the open/close plus fixed slippage ignore market impact and intraday
  liquidity — reasonable for these position sizes, not a guarantee.
- *Short periods.* Annualised figures (CAGR, volatility, Sharpe, Sortino, Calmar, alpha, beta,
  information ratio) are not computed from fewer than 21 trading days and are marked * as
  unreliable below 63 (about three months); even a year of data is a small sample.

## 8. Defaults and presets

| Parameter | Default |
| --- | --- |
| Exposure | 60%–98% by breadth (200-day average); no volatility target |
| Sector budgets | inverse volatility (63 sessions), momentum tilt 0.3 (126/21), trend-aware, floor 3%, cap 22% |
| Companies | inverse volatility (63), momentum tilt 0.4 (126/21), trend penalty 0.5 below the 200-day average (faded in over 10%), cap 5%, minimum 0.5% |
| Rebalancing | band max(0.25 pt, 25% of target), waived beyond a 5 pt portfolio gap; max turnover 30% per plan; minimum trade US$1,000; whole shares |

Presets: **Sector-first risk budget** (the default above), **Equal-weight baseline** (every
company equal, almost fully invested — the yardstick), **Momentum rotation** (strong sectors and
names take more of the budget, names well below trend are scaled out to zero, higher turnover), **Defensive low volatility**
(inverse-variance weights, 20% volatility ceiling, exposure down to 40% in weak markets).
In five-year backtests on the demo market's synthetic history, annual turnover ranges from about
0.4× (equal weight) to about 5× (momentum rotation), with the default near 3×.
Every saved strategy is an immutable version; activations are recorded with who, when and why.
