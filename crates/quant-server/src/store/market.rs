//! Universe reads, price history, quotes and corporate actions.

use std::collections::{BTreeMap, HashMap};

use anyhow::Result;
use chrono::{DateTime, NaiveDate, Utc};
use deadpool_postgres::GenericClient;
use serde::Serialize;

use crate::market::{DailyBar, Dividend, IntradayBar, Quote, Split};

#[derive(Debug, Clone, Serialize)]
pub struct Sector {
    pub id: String,
    pub name_zh: String,
    pub name_en: String,
    pub summary_zh: String,
    pub summary_en: String,
    pub sort_order: i32,
}

#[derive(Debug, Clone, Serialize)]
pub struct Asset {
    pub symbol: String,
    pub name_zh: String,
    pub name_en: String,
    pub sector_id: String,
    pub subtype_id: String,
    pub subtype_zh: String,
    pub subtype_en: String,
    pub also_in: Vec<String>,
    pub role_zh: String,
    pub sort_order: i32,
    pub is_active: bool,
}

pub async fn sectors(client: &impl GenericClient) -> Result<Vec<Sector>> {
    let rows = client
        .query(
            "SELECT id, name_zh, name_en, summary_zh, summary_en, sort_order FROM sectors WHERE is_active ORDER BY sort_order, id",
            &[],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|r| Sector {
            id: r.get(0),
            name_zh: r.get(1),
            name_en: r.get(2),
            summary_zh: r.get(3),
            summary_en: r.get(4),
            sort_order: r.get(5),
        })
        .collect())
}

/// Assets ordered by sector then ontology order; `include_inactive` adds removed members.
pub async fn assets(client: &impl GenericClient, include_inactive: bool) -> Result<Vec<Asset>> {
    let rows = client
        .query(
            "SELECT a.symbol, a.name_zh, a.name_en, a.sector_id, a.subtype_id, a.subtype_zh, a.subtype_en,
                    a.also_in, a.role_zh, a.sort_order, a.is_active
             FROM assets a JOIN sectors s ON s.id = a.sector_id
             WHERE $1 OR a.is_active
             ORDER BY s.sort_order, a.sort_order, a.symbol",
            &[&include_inactive],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|r| Asset {
            symbol: r.get(0),
            name_zh: r.get(1),
            name_en: r.get(2),
            sector_id: r.get(3),
            subtype_id: r.get(4),
            subtype_zh: r.get(5),
            subtype_en: r.get(6),
            also_in: r.get(7),
            role_zh: r.get(8),
            sort_order: r.get(9),
            is_active: r.get(10),
        })
        .collect())
}

pub async fn upsert_daily_bars(client: &impl GenericClient, bars: &[DailyBar]) -> Result<u64> {
    if bars.is_empty() {
        return Ok(0);
    }
    let symbols: Vec<&str> = bars.iter().map(|b| b.symbol.as_str()).collect();
    let dates: Vec<NaiveDate> = bars.iter().map(|b| b.date).collect();
    let open: Vec<f64> = bars.iter().map(|b| b.open).collect();
    let high: Vec<f64> = bars.iter().map(|b| b.high).collect();
    let low: Vec<f64> = bars.iter().map(|b| b.low).collect();
    let close: Vec<f64> = bars.iter().map(|b| b.close).collect();
    let volume: Vec<f64> = bars.iter().map(|b| b.volume).collect();
    let adj_open: Vec<Option<f64>> = bars.iter().map(|b| b.adj_open).collect();
    let adj_close: Vec<Option<f64>> = bars.iter().map(|b| b.adj_close).collect();
    let n = client
        .execute(
            "INSERT INTO daily_bars (symbol, date, open, high, low, close, volume, adj_open, adj_close, updated_at)
             SELECT *, app_now() FROM UNNEST($1::text[], $2::date[], $3::float8[], $4::float8[], $5::float8[],
                                         $6::float8[], $7::float8[], $8::float8[], $9::float8[])
             ON CONFLICT (symbol, date) DO UPDATE SET open = EXCLUDED.open, high = EXCLUDED.high,
               low = EXCLUDED.low, close = EXCLUDED.close, volume = EXCLUDED.volume,
               adj_open = COALESCE(EXCLUDED.adj_open, daily_bars.adj_open),
               adj_close = COALESCE(EXCLUDED.adj_close, daily_bars.adj_close),
               updated_at = app_now()",
            &[&symbols, &dates, &open, &high, &low, &close, &volume, &adj_open, &adj_close],
        )
        .await?;
    Ok(n)
}

pub async fn daily_bars(
    client: &impl GenericClient,
    symbol: &str,
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
) -> Result<Vec<DailyBar>> {
    let rows = client
        .query(
            "SELECT symbol, date, open, high, low, close, volume, adj_open, adj_close FROM daily_bars
             WHERE symbol = $1 AND ($2::date IS NULL OR date >= $2) AND ($3::date IS NULL OR date <= $3)
             ORDER BY date",
            &[&symbol, &from, &to],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|r| DailyBar {
            symbol: r.get(0),
            date: r.get(1),
            open: r.get(2),
            high: r.get(3),
            low: r.get(4),
            close: r.get(5),
            volume: r.get(6),
            adj_open: r.get(7),
            adj_close: r.get(8),
        })
        .collect())
}

/// Adjusted daily panel for many symbols since `from`: per symbol, (date, adj_open, adj_close).
/// Falls back to unadjusted prices where the provider gave no adjusted values.
pub async fn adjusted_panel(
    client: &impl GenericClient,
    symbols: &[String],
    from: NaiveDate,
    to: Option<NaiveDate>,
) -> Result<HashMap<String, Vec<(NaiveDate, f64, f64)>>> {
    let rows = client
        .query(
            "SELECT symbol, date, COALESCE(adj_open, open), COALESCE(adj_close, close) FROM daily_bars
             WHERE symbol = ANY($1) AND date >= $2 AND ($3::date IS NULL OR date <= $3)
             ORDER BY symbol, date",
            &[&symbols, &from, &to],
        )
        .await?;
    let mut out: HashMap<String, Vec<(NaiveDate, f64, f64)>> = HashMap::new();
    for r in rows {
        out.entry(r.get(0))
            .or_default()
            .push((r.get(1), r.get(2), r.get(3)));
    }
    Ok(out)
}

#[derive(Debug, Clone, Serialize)]
pub struct Coverage {
    pub symbol: String,
    pub first: Option<NaiveDate>,
    pub last: Option<NaiveDate>,
    pub bars: i64,
    pub adjusted: i64,
}

pub async fn coverage(client: &impl GenericClient) -> Result<Vec<Coverage>> {
    let rows = client
        .query(
            "SELECT symbol, min(date), max(date), count(*), count(adj_close) FROM daily_bars GROUP BY symbol ORDER BY symbol",
            &[],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|r| Coverage {
            symbol: r.get(0),
            first: r.get(1),
            last: r.get(2),
            bars: r.get(3),
            adjusted: r.get(4),
        })
        .collect())
}

pub async fn last_bar_dates(client: &impl GenericClient) -> Result<BTreeMap<String, NaiveDate>> {
    let rows = client
        .query(
            "SELECT symbol, max(date) FROM daily_bars GROUP BY symbol",
            &[],
        )
        .await?;
    Ok(rows.iter().map(|r| (r.get(0), r.get(1))).collect())
}

/// The latest complete close before `date` for each symbol (for change baselines and guards).
pub async fn closes_before(
    client: &impl GenericClient,
    symbols: &[String],
    date: NaiveDate,
) -> Result<HashMap<String, (NaiveDate, f64)>> {
    let rows = client
        .query(
            "SELECT DISTINCT ON (symbol) symbol, date, close FROM daily_bars
             WHERE symbol = ANY($1) AND date < $2 ORDER BY symbol, date DESC",
            &[&symbols, &date],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|r| (r.get(0), (r.get(1), r.get(2))))
        .collect())
}

pub async fn upsert_quotes(client: &impl GenericClient, quotes: &[Quote]) -> Result<()> {
    for q in quotes {
        client
            .execute(
                "INSERT INTO quotes (symbol, price, change, change_pct, open, day_high, day_low, prev_close, volume, avg_volume, market_cap, quote_ts, fetched_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, app_now())
                 ON CONFLICT (symbol) DO UPDATE SET price = EXCLUDED.price, change = EXCLUDED.change,
                   change_pct = EXCLUDED.change_pct, open = EXCLUDED.open, day_high = EXCLUDED.day_high,
                   day_low = EXCLUDED.day_low, prev_close = EXCLUDED.prev_close, volume = EXCLUDED.volume,
                   avg_volume = EXCLUDED.avg_volume, market_cap = EXCLUDED.market_cap,
                   quote_ts = EXCLUDED.quote_ts, fetched_at = app_now()",
                &[
                    &q.symbol,
                    &q.price,
                    &q.change,
                    &q.change_pct,
                    &q.open,
                    &q.day_high,
                    &q.day_low,
                    &q.prev_close,
                    &q.volume,
                    &q.avg_volume,
                    &q.market_cap,
                    &q.timestamp,
                ],
            )
            .await?;
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct StoredQuote {
    #[serde(flatten)]
    pub quote: Quote,
    pub fetched_at: DateTime<Utc>,
}

pub async fn quotes(client: &impl GenericClient) -> Result<HashMap<String, StoredQuote>> {
    let rows = client
        .query(
            "SELECT symbol, price, change, change_pct, open, day_high, day_low, prev_close, volume, avg_volume, market_cap, quote_ts, fetched_at FROM quotes",
            &[],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|r| {
            let symbol: String = r.get(0);
            (
                symbol.clone(),
                StoredQuote {
                    quote: Quote {
                        symbol,
                        price: r.get(1),
                        change: r.get(2),
                        change_pct: r.get(3),
                        open: r.get(4),
                        day_high: r.get(5),
                        day_low: r.get(6),
                        prev_close: r.get(7),
                        volume: r.get(8),
                        avg_volume: r.get(9),
                        market_cap: r.get(10),
                        timestamp: r.get(11),
                    },
                    fetched_at: r.get(12),
                },
            )
        })
        .collect())
}

pub async fn replace_intraday(
    client: &impl GenericClient,
    symbol: &str,
    interval: &str,
    bars: &[IntradayBar],
) -> Result<()> {
    if bars.is_empty() {
        return Ok(());
    }
    let ts: Vec<DateTime<Utc>> = bars.iter().map(|b| b.ts).collect();
    let open: Vec<f64> = bars.iter().map(|b| b.open).collect();
    let high: Vec<f64> = bars.iter().map(|b| b.high).collect();
    let low: Vec<f64> = bars.iter().map(|b| b.low).collect();
    let close: Vec<f64> = bars.iter().map(|b| b.close).collect();
    let volume: Vec<f64> = bars.iter().map(|b| b.volume).collect();
    client
        .execute(
            "INSERT INTO intraday_bars (symbol, interval, ts, open, high, low, close, volume, fetched_at)
             SELECT $1, $2, t.*, app_now() FROM UNNEST($3::timestamptz[], $4::float8[], $5::float8[], $6::float8[], $7::float8[], $8::float8[]) AS t
             ON CONFLICT (symbol, interval, ts) DO UPDATE SET open = EXCLUDED.open, high = EXCLUDED.high,
               low = EXCLUDED.low, close = EXCLUDED.close, volume = EXCLUDED.volume, fetched_at = app_now()",
            &[&symbol, &interval, &ts, &open, &high, &low, &close, &volume],
        )
        .await?;
    Ok(())
}

pub async fn intraday(
    client: &impl GenericClient,
    symbol: &str,
    interval: &str,
    from: DateTime<Utc>,
) -> Result<(Vec<IntradayBar>, Option<DateTime<Utc>>)> {
    let rows = client
        .query(
            "SELECT ts, open, high, low, close, volume, fetched_at FROM intraday_bars
             WHERE symbol = $1 AND interval = $2 AND ts >= $3 ORDER BY ts",
            &[&symbol, &interval, &from],
        )
        .await?;
    let fetched = rows.iter().map(|r| r.get::<_, DateTime<Utc>>(6)).max();
    Ok((
        rows.iter()
            .map(|r| IntradayBar {
                ts: r.get(0),
                open: r.get(1),
                high: r.get(2),
                low: r.get(3),
                close: r.get(4),
                volume: r.get(5),
            })
            .collect(),
        fetched,
    ))
}

pub async fn upsert_splits(client: &impl GenericClient, splits: &[Split]) -> Result<()> {
    for s in splits {
        client
            .execute(
                "INSERT INTO corporate_actions (symbol, kind, ex_date, ratio) VALUES ($1, 'split', $2, $3)
                 ON CONFLICT (symbol, kind, ex_date) DO UPDATE SET ratio = EXCLUDED.ratio, fetched_at = app_now()",
                &[&s.symbol, &s.date, &s.ratio()],
            )
            .await?;
    }
    Ok(())
}

pub async fn upsert_dividends(client: &impl GenericClient, dividends: &[Dividend]) -> Result<()> {
    for d in dividends {
        client
            .execute(
                "INSERT INTO corporate_actions (symbol, kind, ex_date, pay_date, amount) VALUES ($1, 'dividend', $2, $3, $4)
                 ON CONFLICT (symbol, kind, ex_date) DO UPDATE SET pay_date = EXCLUDED.pay_date, amount = EXCLUDED.amount, fetched_at = app_now()",
                &[&d.symbol, &d.ex_date, &d.pay_date, &d.amount],
            )
            .await?;
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct CorporateAction {
    pub id: i64,
    pub symbol: String,
    pub kind: String,
    pub ex_date: NaiveDate,
    pub pay_date: Option<NaiveDate>,
    pub ratio: Option<f64>,
    pub amount: Option<f64>,
}

/// Actions with an ex-date in `[from, to]` not yet applied to the account.
pub async fn pending_actions(
    client: &impl GenericClient,
    account_id: i64,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<CorporateAction>> {
    let rows = client
        .query(
            "SELECT c.id, c.symbol, c.kind, c.ex_date, c.pay_date, c.ratio, c.amount
             FROM corporate_actions c
             WHERE c.ex_date BETWEEN $2 AND $3
               AND NOT EXISTS (SELECT 1 FROM corporate_action_applications a WHERE a.account_id = $1 AND a.action_id = c.id)
             ORDER BY c.ex_date, c.kind DESC, c.symbol",
            &[&account_id, &from, &to],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|r| CorporateAction {
            id: r.get(0),
            symbol: r.get(1),
            kind: r.get(2),
            ex_date: r.get(3),
            pay_date: r.get(4),
            ratio: r.get(5),
            amount: r.get(6),
        })
        .collect())
}

/// Raw (split-adjusted) daily OHLCV for many symbols in `[from, to]`, oldest first.
pub async fn daily_ohlc_panel(
    client: &impl GenericClient,
    symbols: &[String],
    from: NaiveDate,
    to: NaiveDate,
) -> Result<HashMap<String, Vec<DailyBar>>> {
    let rows = client
        .query(
            "SELECT symbol, date, open, high, low, close, volume, adj_open, adj_close FROM daily_bars
             WHERE symbol = ANY($1) AND date BETWEEN $2 AND $3 ORDER BY symbol, date",
            &[&symbols, &from, &to],
        )
        .await?;
    let mut out: HashMap<String, Vec<DailyBar>> = HashMap::new();
    for r in rows {
        let bar = DailyBar {
            symbol: r.get(0),
            date: r.get(1),
            open: r.get(2),
            high: r.get(3),
            low: r.get(4),
            close: r.get(5),
            volume: r.get(6),
            adj_open: r.get(7),
            adj_close: r.get(8),
        };
        out.entry(bar.symbol.clone()).or_default().push(bar);
    }
    Ok(out)
}
