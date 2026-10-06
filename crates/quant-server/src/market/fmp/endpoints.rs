//! FMP endpoint catalogue: request paths for both API generations, symbol validation, and the
//! rules that split one logical request into several HTTP requests (quote batches, multi-year
//! daily ranges).
//!
//! Symbols are the only caller-supplied text that reaches a URL, so they are validated against a
//! strict ticker alphabet and percent-encoded; everything else is a date or a fixed token.

use std::fmt;

use chrono::{Months, NaiveDate};

use crate::market::Interval;

/// Which FMP API generation serves a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum ApiKind {
    /// `/stable/*`, available to every current subscription.
    Stable,
    /// `/api/v3/*`, only available to subscriptions opened before August 2025.
    Legacy,
}

impl ApiKind {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            ApiKind::Stable => "stable",
            ApiKind::Legacy => "legacy",
        }
    }
}

/// Endpoint families whose availability can differ between API generations and plans; `Auto`
/// mode remembers per family which generation answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Family {
    Quote,
    Daily,
    /// Split- and dividend-adjusted (total return) daily prices.
    Adjusted,
    Intraday,
    Splits,
    Dividends,
}

impl Family {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Family::Quote => "quote",
            Family::Daily => "daily",
            Family::Adjusted => "adjusted",
            Family::Intraday => "intraday",
            Family::Splits => "splits",
            Family::Dividends => "dividends",
        }
    }
}

/// Most symbols FMP accepts in one quote request.
pub(super) const MAX_BATCH_SYMBOLS: usize = 25;
/// Upper bound for the encoded, comma-joined symbol list of one quote request.
pub(super) const MAX_BATCH_CHARS: usize = 700;
/// No listed ticker comes close; the cap keeps garbage out of URLs and batches.
const MAX_SYMBOL_LEN: usize = 32;
/// Some plans cap the history one daily request returns (about five years), so longer ranges
/// are fetched in chunks of at most four years.
const MAX_DAILY_SPAN: Months = Months::new(48);

/// One FMP request without its API key. `Display` renders path and query, which is therefore
/// always safe to log or put in an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Request {
    /// Percent-encoded path, e.g. `/stable/batch-quote`.
    path: String,
    /// `name=value` pairs whose values are already percent-encoded.
    query: Vec<(&'static str, String)>,
}

impl Request {
    fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            query: Vec::new(),
        }
    }

    /// Adds a query parameter whose value is already percent-encoded.
    fn param(mut self, name: &'static str, encoded: impl Into<String>) -> Self {
        self.query.push((name, encoded.into()));
        self
    }

    fn range(self, from: NaiveDate, to: NaiveDate) -> Self {
        self.param("from", from.format("%Y-%m-%d").to_string())
            .param("to", to.format("%Y-%m-%d").to_string())
    }

    /// The full URL including the API key. Never log or display the result.
    pub(super) fn url(&self, base_url: &str, key: &str) -> String {
        let separator = if self.query.is_empty() { '?' } else { '&' };
        format!(
            "{base_url}{self}{separator}apikey={}",
            urlencoding::encode(key)
        )
    }
}

impl fmt::Display for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.path)?;
        for (index, (name, value)) in self.query.iter().enumerate() {
            let separator = if index == 0 { '?' } else { '&' };
            write!(f, "{separator}{name}={value}")?;
        }
        Ok(())
    }
}

/// Upper-cases `raw` and accepts it when it can be a ticker: `[A-Za-z0-9.\-_^]+` with at least
/// one letter or digit and at most 32 characters. Anything else could change the URL's meaning.
pub(super) fn normalize_symbol(raw: &str) -> Option<String> {
    let symbol = raw.trim().to_ascii_uppercase();
    let valid = !symbol.is_empty()
        && symbol.len() <= MAX_SYMBOL_LEN
        && symbol
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b'^'))
        && symbol.bytes().any(|b| b.is_ascii_alphanumeric());
    valid.then_some(symbol)
}

fn encode(symbol: &str) -> String {
    urlencoding::encode(symbol).into_owned()
}

/// Comma-joined symbols, each percent-encoded; the commas stay literal because FMP splits on them.
fn encode_list(symbols: &[String]) -> String {
    symbols
        .iter()
        .map(|symbol| encode(symbol))
        .collect::<Vec<_>>()
        .join(",")
}

/// Splits symbols into quote batches of at most [`MAX_BATCH_SYMBOLS`] whose encoded,
/// comma-joined form stays within [`MAX_BATCH_CHARS`].
pub(super) fn quote_batches(symbols: &[String]) -> Vec<Vec<String>> {
    let mut batches = Vec::new();
    let mut current: Vec<String> = Vec::new();
    let mut chars = 0;
    for symbol in symbols {
        let len = encode(symbol).len();
        if !current.is_empty()
            && (current.len() == MAX_BATCH_SYMBOLS || chars + 1 + len > MAX_BATCH_CHARS)
        {
            batches.push(std::mem::take(&mut current));
        }
        chars = if current.is_empty() {
            len
        } else {
            chars + 1 + len
        };
        current.push(symbol.clone());
    }
    if !current.is_empty() {
        batches.push(current);
    }
    batches
}

/// Splits `[from, to]` into consecutive, non-overlapping ranges of at most four years.
pub(super) fn date_chunks(from: NaiveDate, to: NaiveDate) -> Vec<(NaiveDate, NaiveDate)> {
    let mut chunks = Vec::new();
    let mut start = from;
    while start <= to {
        let end = start
            .checked_add_months(MAX_DAILY_SPAN)
            .and_then(|date| date.pred_opt())
            .map_or(to, |date| date.min(to));
        chunks.push((start, end));
        match end.succ_opt() {
            Some(next) => start = next,
            None => break,
        }
    }
    chunks
}

/// Latest quotes. A single symbol uses the single-quote endpoint, which more plans include.
pub(super) fn quotes(api: ApiKind, symbols: &[String]) -> Request {
    let list = encode_list(symbols);
    match (api, symbols.len()) {
        (ApiKind::Stable, 1) => Request::new("/stable/quote").param("symbol", list),
        (ApiKind::Stable, _) => Request::new("/stable/batch-quote").param("symbols", list),
        (ApiKind::Legacy, _) => Request::new(format!("/api/v3/quote/{list}")),
    }
}

/// Split-adjusted daily OHLCV. Legacy rows also carry `adjClose`.
pub(super) fn daily(api: ApiKind, symbol: &str, from: NaiveDate, to: NaiveDate) -> Request {
    match api {
        ApiKind::Stable => Request::new("/stable/historical-price-eod/full")
            .param("symbol", encode(symbol))
            .range(from, to),
        ApiKind::Legacy => {
            Request::new(format!("/api/v3/historical-price-full/{}", encode(symbol)))
                .range(from, to)
        }
    }
}

/// Split- and dividend-adjusted daily prices. Legacy has no dedicated endpoint: its daily rows
/// carry `adjClose`.
pub(super) fn adjusted(api: ApiKind, symbol: &str, from: NaiveDate, to: NaiveDate) -> Request {
    match api {
        ApiKind::Stable => Request::new("/stable/historical-price-eod/dividend-adjusted")
            .param("symbol", encode(symbol))
            .range(from, to),
        ApiKind::Legacy => daily(ApiKind::Legacy, symbol, from, to),
    }
}

/// Intraday bars, stamped in New York wall-clock time.
pub(super) fn intraday(
    api: ApiKind,
    symbol: &str,
    interval: Interval,
    from: NaiveDate,
    to: NaiveDate,
) -> Request {
    match api {
        ApiKind::Stable => Request::new(format!("/stable/historical-chart/{}", interval.as_str()))
            .param("symbol", encode(symbol))
            .range(from, to),
        ApiKind::Legacy => Request::new(format!(
            "/api/v3/historical-chart/{}/{}",
            interval.as_str(),
            encode(symbol)
        ))
        .range(from, to),
    }
}

pub(super) fn splits(api: ApiKind, symbol: &str) -> Request {
    match api {
        ApiKind::Stable => Request::new("/stable/splits").param("symbol", encode(symbol)),
        ApiKind::Legacy => Request::new(format!(
            "/api/v3/historical-price-full/stock_split/{}",
            encode(symbol)
        )),
    }
}

pub(super) fn dividends(api: ApiKind, symbol: &str) -> Request {
    match api {
        ApiKind::Stable => Request::new("/stable/dividends").param("symbol", encode(symbol)),
        ApiKind::Legacy => Request::new(format!(
            "/api/v3/historical-price-full/stock_dividend/{}",
            encode(symbol)
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn owned(symbols: &[&str]) -> Vec<String> {
        symbols.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn symbols_are_upper_cased_and_validated() {
        assert_eq!(normalize_symbol(" aapl ").as_deref(), Some("AAPL"));
        assert_eq!(normalize_symbol("brk-b").as_deref(), Some("BRK-B"));
        assert_eq!(normalize_symbol("BRK.B").as_deref(), Some("BRK.B"));
        assert_eq!(normalize_symbol("^gspc").as_deref(), Some("^GSPC"));
        assert_eq!(normalize_symbol("ABC_D").as_deref(), Some("ABC_D"));
        for bad in [
            "",
            "   ",
            "BRK B",
            "A&B",
            "A/B",
            "A?B",
            "A#B",
            "AAPL,MSFT",
            "..",
            "^",
            "ÄPPLE",
        ] {
            assert_eq!(normalize_symbol(bad), None, "{bad:?} must be rejected");
        }
        assert_eq!(normalize_symbol(&"A".repeat(33)), None);
        assert!(normalize_symbol(&"A".repeat(32)).is_some());
    }

    #[test]
    fn display_never_contains_the_key_but_the_url_does() {
        let request = quotes(ApiKind::Stable, &owned(&["AAPL", "^GSPC"]));
        assert_eq!(
            request.to_string(),
            "/stable/batch-quote?symbols=AAPL,%5EGSPC"
        );
        assert_eq!(
            request.url("https://fmp.test", "k/1"),
            "https://fmp.test/stable/batch-quote?symbols=AAPL,%5EGSPC&apikey=k%2F1"
        );
        let legacy = splits(ApiKind::Legacy, "AAPL");
        assert_eq!(
            legacy.url("https://fmp.test", "key"),
            "https://fmp.test/api/v3/historical-price-full/stock_split/AAPL?apikey=key"
        );
    }

    #[test]
    fn endpoint_paths_for_both_generations() {
        let (from, to) = (d(2025, 1, 2), d(2025, 3, 31));
        let one = owned(&["AAPL"]);
        let cases = [
            (quotes(ApiKind::Stable, &one), "/stable/quote?symbol=AAPL"),
            (
                quotes(ApiKind::Legacy, &owned(&["AAPL", "MSFT"])),
                "/api/v3/quote/AAPL,MSFT",
            ),
            (
                daily(ApiKind::Stable, "AAPL", from, to),
                "/stable/historical-price-eod/full?symbol=AAPL&from=2025-01-02&to=2025-03-31",
            ),
            (
                daily(ApiKind::Legacy, "AAPL", from, to),
                "/api/v3/historical-price-full/AAPL?from=2025-01-02&to=2025-03-31",
            ),
            (
                adjusted(ApiKind::Stable, "AAPL", from, to),
                "/stable/historical-price-eod/dividend-adjusted?symbol=AAPL&from=2025-01-02&to=2025-03-31",
            ),
            (
                adjusted(ApiKind::Legacy, "AAPL", from, to),
                "/api/v3/historical-price-full/AAPL?from=2025-01-02&to=2025-03-31",
            ),
            (
                intraday(ApiKind::Stable, "AAPL", Interval::FiveMin, from, to),
                "/stable/historical-chart/5min?symbol=AAPL&from=2025-01-02&to=2025-03-31",
            ),
            (
                intraday(ApiKind::Legacy, "^GSPC", Interval::OneHour, from, to),
                "/api/v3/historical-chart/1hour/%5EGSPC?from=2025-01-02&to=2025-03-31",
            ),
            (
                splits(ApiKind::Stable, "AAPL"),
                "/stable/splits?symbol=AAPL",
            ),
            (
                dividends(ApiKind::Stable, "AAPL"),
                "/stable/dividends?symbol=AAPL",
            ),
            (
                dividends(ApiKind::Legacy, "AAPL"),
                "/api/v3/historical-price-full/stock_dividend/AAPL",
            ),
        ];
        for (request, expected) in cases {
            assert_eq!(request.to_string(), expected);
        }
    }

    #[test]
    fn quote_batches_cap_symbol_count() {
        let symbols: Vec<String> = (0..60).map(|i| format!("S{i}")).collect();
        let batches = quote_batches(&symbols);
        let sizes: Vec<usize> = batches.iter().map(Vec::len).collect();
        assert_eq!(sizes, vec![25, 25, 10]);
        assert_eq!(batches.concat(), symbols, "order is preserved");
    }

    #[test]
    fn quote_batches_cap_encoded_length() {
        // 30 symbols of 30 characters: 22 fit in 700 characters (22 * 30 + 21 commas = 681).
        let symbols: Vec<String> = (0..30).map(|i| format!("{i:A>30}")).collect();
        let batches = quote_batches(&symbols);
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].len(), 22);
        for batch in &batches {
            assert!(encode_list(batch).len() <= MAX_BATCH_CHARS);
        }
        // `^` is encoded as `%5E`, so encoded length is what counts.
        let carets: Vec<String> = (0..25).map(|i| format!("^{i:X>29}")).collect();
        for batch in quote_batches(&carets) {
            assert!(encode_list(&batch).len() <= MAX_BATCH_CHARS);
        }
        assert!(quote_batches(&[]).is_empty());
    }

    #[test]
    fn short_daily_ranges_are_a_single_chunk() {
        assert_eq!(
            date_chunks(d(2020, 1, 1), d(2023, 12, 31)),
            vec![(d(2020, 1, 1), d(2023, 12, 31))]
        );
        assert_eq!(
            date_chunks(d(2025, 1, 2), d(2025, 1, 2)),
            vec![(d(2025, 1, 2), d(2025, 1, 2))]
        );
        assert!(date_chunks(d(2025, 1, 3), d(2025, 1, 2)).is_empty());
    }

    #[test]
    fn long_daily_ranges_split_into_four_year_chunks() {
        assert_eq!(
            date_chunks(d(2010, 1, 1), d(2021, 6, 30)),
            vec![
                (d(2010, 1, 1), d(2013, 12, 31)),
                (d(2014, 1, 1), d(2017, 12, 31)),
                (d(2018, 1, 1), d(2021, 6, 30)),
            ]
        );
        // Leap-day start: chunks stay contiguous and within four years.
        let chunks = date_chunks(d(2016, 2, 29), d(2026, 10, 5));
        assert_eq!(chunks.first().unwrap().0, d(2016, 2, 29));
        assert_eq!(chunks.last().unwrap().1, d(2026, 10, 5));
        for pair in chunks.windows(2) {
            assert_eq!(pair[0].1.succ_opt().unwrap(), pair[1].0);
        }
        for (start, end) in chunks {
            assert!(end < start.checked_add_months(Months::new(48)).unwrap());
        }
    }
}
