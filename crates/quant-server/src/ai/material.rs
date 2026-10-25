//! What the AI analyst reads: a compact, dated digest of honeclaw's ontology per sector, objective
//! statistics computed from adjusted closes, and reported earnings against consensus.
//!
//! Everything here is a pure function of its inputs and only uses information dated on or before
//! `as_of` (the last close), so the material a view was formed from can be stored and re-read.
//! Text is cut to fixed lengths: the whole briefing has to fit one modest request a day.

use std::collections::{BTreeMap, HashMap};

use chrono::NaiveDate;
use serde::Serialize;
use serde_json::{Value, json};

use super::llm::truncate;
use crate::market::EarningsReport;

/// Material older than this (days before `as_of`) is marked stale.
pub const STALE_DAYS: i64 = 120;

/// One sector of the universe with its members, as hone-quant trades it.
#[derive(Debug, Clone)]
pub struct SectorMembers {
    pub id: String,
    pub name_zh: String,
    pub name_en: String,
    pub symbols: Vec<String>,
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn cut(value: &Value, key: &str, max: usize) -> String {
    truncate(&text(value, key), max)
}

fn date_of(value: &Value, key: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(&text(value, key), "%Y-%m-%d").ok()
}

fn list<'a>(value: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    value
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

/// Dated items only count when they are not after `as_of`; undated ones are kept.
fn not_after(date: Option<NaiveDate>, as_of: NaiveDate) -> bool {
    date.is_none_or(|d| d <= as_of)
}

/// The ontology's research on each sector of the universe, newest first and cut to size.
/// Sectors the ontology does not describe get an empty entry, so the analyst sees the gap.
pub fn ontology_digest(ontology: &Value, sectors: &[SectorMembers], as_of: NaiveDate) -> Value {
    let industries: HashMap<String, &Value> = list(ontology, "industries")
        .map(|i| (text(i, "id"), i))
        .collect();
    // A member's role, wherever the ontology lists it.
    let mut roles: HashMap<String, String> = HashMap::new();
    for industry in industries.values() {
        for member in list(industry, "members") {
            roles
                .entry(text(member, "symbol"))
                .or_insert_with(|| cut(member, "role", 70));
        }
    }
    let digest: Vec<Value> = sectors
        .iter()
        .map(|sector| {
            let members: Vec<Value> = sector
                .symbols
                .iter()
                .map(|s| json!({"symbol": s, "role": roles.get(s).cloned().unwrap_or_default()}))
                .collect();
            let Some(industry) = industries.get(&sector.id) else {
                return json!({"sector": sector.id, "name": sector.name_zh, "members": members, "research": null});
            };
            let brief = industry.get("brief").cloned().unwrap_or(Value::Null);
            let logic = industry
                .get("ai_valuation_logic")
                .cloned()
                .unwrap_or(Value::Null);
            let key_variables: Vec<Value> = list(&logic, "key_variables")
                .filter(|v| not_after(date_of(v, "as_of"), as_of))
                .take(6)
                .map(|v| json!({"name": cut(v, "name", 60), "as_of": text(v, "as_of"), "why": cut(v, "why", 140)}))
                .collect();
            let signals: Vec<Value> = list(industry, "upstream_signals")
                .filter(|v| not_after(date_of(v, "latest_as_of"), as_of))
                .take(5)
                .map(|v| {
                    json!({
                        "from": format!("{} {}", text(v, "symbol"), text(v, "name")).trim().to_string(),
                        "relation": text(v, "relation"),
                        "latest": cut(v, "latest", 220),
                        "as_of": text(v, "latest_as_of"),
                    })
                })
                .collect();
            let watch: Vec<Value> = list(industry, "core_watch")
                .filter(|v| not_after(date_of(v, "as_of"), as_of))
                .take(4)
                .map(|v| json!({"what": cut(v, "what", 110), "cadence": cut(v, "cadence", 70), "as_of": text(v, "as_of")}))
                .collect();
            let mut sources: Vec<&Value> = list(industry, "sources")
                .filter(|v| date_of(v, "date").is_some_and(|d| d <= as_of))
                .collect();
            sources.sort_by_key(|v| std::cmp::Reverse(date_of(v, "date")));
            let sources: Vec<Value> = sources
                .into_iter()
                .take(4)
                .map(|v| json!({"date": text(v, "date"), "house": cut(v, "house", 40), "takeaway": cut(v, "takeaway", 220)}))
                .collect();
            let brief_date = date_of(&brief, "as_of");
            let brief = if not_after(brief_date, as_of) {
                json!({
                    "as_of": text(&brief, "as_of"),
                    "question": cut(&brief, "question", 160),
                    "body": cut(&brief, "body", 480),
                    "next": list(&brief, "next").filter_map(Value::as_str).take(3).map(|n| truncate(n, 140)).collect::<Vec<_>>(),
                })
            } else {
                Value::Null
            };
            let newest = [brief_date]
                .into_iter()
                .chain(sources.iter().map(|v| date_of(v, "date")))
                .flatten()
                .max();
            json!({
                "sector": sector.id,
                "name": sector.name_zh,
                "members": members,
                "research": {
                    "one_liner": cut(industry, "one_liner", 120),
                    "brief": brief,
                    "driver_chain": cut(&logic, "driver_chain", 420),
                    "anti_pattern": cut(&logic, "anti_pattern_short", 200),
                    "key_variables": key_variables,
                    "upstream_signals": signals,
                    "core_watch": watch,
                    "recent_sources": sources,
                    "newest_material": newest,
                    "stale": newest.is_none_or(|d| (as_of - d).num_days() > STALE_DAYS),
                },
            })
        })
        .collect();
    json!({
        "ontology_version": ontology.get("schema_version"),
        "ontology_generated_at": ontology.get("generated_at"),
        "sectors": digest,
    })
}

// ---------------------------------------------------------------------------------------------
// Objective market statistics
// ---------------------------------------------------------------------------------------------

/// Statistics of one price series ending at `as_of` (percent, rounded).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct SeriesStats {
    pub ret_1m: Option<f64>,
    pub ret_3m: Option<f64>,
    pub ret_6m: Option<f64>,
    pub ret_12m: Option<f64>,
    /// Annualised volatility of daily returns over three months.
    pub vol_3m: Option<f64>,
    /// Distance to the 200-day average.
    pub vs_200d: Option<f64>,
    /// Distance to the highest close of the last year.
    pub from_high: Option<f64>,
}

fn pct(x: f64) -> f64 {
    (x * 1000.0).round() / 10.0
}

fn ret(closes: &[f64], days: usize) -> Option<f64> {
    let n = closes.len();
    (n > days).then(|| pct(closes[n - 1] / closes[n - 1 - days] - 1.0))
}

pub fn series_stats(closes: &[f64]) -> SeriesStats {
    let n = closes.len();
    if n < 2 {
        return SeriesStats::default();
    }
    let last = closes[n - 1];
    let vol_3m = (n > 63).then(|| {
        let r: Vec<f64> = closes[n - 64..]
            .windows(2)
            .map(|w| w[1] / w[0] - 1.0)
            .collect();
        let mean = r.iter().sum::<f64>() / r.len() as f64;
        let var = r.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (r.len() - 1) as f64;
        pct((var * 252.0).sqrt())
    });
    let vs_200d =
        (n >= 200).then(|| pct(last / (closes[n - 200..].iter().sum::<f64>() / 200.0) - 1.0));
    let window = &closes[n.saturating_sub(252)..];
    let high = window.iter().cloned().fold(f64::MIN, f64::max);
    SeriesStats {
        ret_1m: ret(closes, 21),
        ret_3m: ret(closes, 63),
        ret_6m: ret(closes, 126),
        ret_12m: ret(closes, 252),
        vol_3m,
        vs_200d,
        from_high: Some(pct(last / high - 1.0)),
    }
}

/// An equal-weight index of members' daily returns over the dates they share, starting at 1.
fn equal_weight_index(series: &[&[(NaiveDate, f64)]]) -> Vec<f64> {
    let mut by_date: BTreeMap<NaiveDate, (f64, usize)> = BTreeMap::new();
    for s in series {
        for w in s.windows(2) {
            if w[0].1 > 0.0 && w[1].1 > 0.0 {
                let slot = by_date.entry(w[1].0).or_insert((0.0, 0));
                slot.0 += w[1].1 / w[0].1 - 1.0;
                slot.1 += 1;
            }
        }
    }
    let mut level = 1.0;
    let mut index = vec![level];
    for (sum, count) in by_date.values() {
        level *= 1.0 + sum / *count as f64;
        index.push(level);
    }
    index
}

/// Per sector: its equal-weight index statistics, the share of members above their 200-day
/// average and each member's own statistics; plus the benchmarks.
pub fn market_stats(
    sectors: &[SectorMembers],
    closes: &HashMap<String, Vec<(NaiveDate, f64)>>,
    benchmarks: &[String],
) -> Value {
    let empty: Vec<(NaiveDate, f64)> = Vec::new();
    let only_prices = |s: &[(NaiveDate, f64)]| s.iter().map(|(_, c)| *c).collect::<Vec<f64>>();
    let sectors: Vec<Value> = sectors
        .iter()
        .map(|sector| {
            let series: Vec<&[(NaiveDate, f64)]> = sector
                .symbols
                .iter()
                .map(|s| closes.get(s).unwrap_or(&empty).as_slice())
                .collect();
            let members: Vec<Value> = sector
                .symbols
                .iter()
                .zip(&series)
                .map(|(symbol, s)| {
                    let st = series_stats(&only_prices(s));
                    json!({"symbol": symbol, "ret_1m": st.ret_1m, "ret_3m": st.ret_3m, "ret_12m": st.ret_12m, "vol_3m": st.vol_3m, "vs_200d": st.vs_200d})
                })
                .collect();
            let gaps: Vec<f64> = series
                .iter()
                .filter_map(|s| series_stats(&only_prices(s)).vs_200d)
                .collect();
            let above = (!gaps.is_empty())
                .then(|| pct(gaps.iter().filter(|g| **g > 0.0).count() as f64 / gaps.len() as f64));
            json!({
                "sector": sector.id,
                "index": series_stats(&equal_weight_index(&series)),
                "share_above_200d": above,
                "members": members,
            })
        })
        .collect();
    let benchmarks: Vec<Value> = benchmarks
        .iter()
        .map(|b| {
            let st = series_stats(&only_prices(closes.get(b).unwrap_or(&empty)));
            json!({"symbol": b, "ret_1m": st.ret_1m, "ret_3m": st.ret_3m, "ret_6m": st.ret_6m, "ret_12m": st.ret_12m})
        })
        .collect();
    json!({"unit": "percent", "sectors": sectors, "benchmarks": benchmarks})
}

// ---------------------------------------------------------------------------------------------
// Earnings
// ---------------------------------------------------------------------------------------------

fn surprise(actual: Option<f64>, estimate: Option<f64>) -> Option<f64> {
    match (actual, estimate) {
        (Some(a), Some(e)) if e.abs() > 1e-9 => Some(pct((a - e) / e.abs())),
        _ => None,
    }
}

/// Per member: the latest report on or before `as_of` (within [`STALE_DAYS`]) against consensus,
/// and the next scheduled date within 60 days.
pub fn earnings_summary(
    sectors: &[SectorMembers],
    reports: &HashMap<String, Vec<EarningsReport>>,
    as_of: NaiveDate,
) -> Value {
    let sectors: Vec<Value> = sectors
        .iter()
        .map(|sector| {
            let mut beats = 0;
            let mut misses = 0;
            let members: Vec<Value> = sector
                .symbols
                .iter()
                .filter_map(|symbol| {
                    let list = reports.get(symbol)?;
                    let last = list.iter().rev().find(|r| {
                        r.date <= as_of
                            && (as_of - r.date).num_days() <= STALE_DAYS
                            && (r.eps_actual.is_some() || r.revenue_actual.is_some())
                    });
                    let next = list
                        .iter()
                        .find(|r| r.date > as_of && (r.date - as_of).num_days() <= 60);
                    if last.is_none() && next.is_none() {
                        return None;
                    }
                    let eps = last.and_then(|r| surprise(r.eps_actual, r.eps_estimate));
                    let revenue = last.and_then(|r| surprise(r.revenue_actual, r.revenue_estimate));
                    match revenue.or(eps) {
                        Some(s) if s > 0.0 => beats += 1,
                        Some(s) if s < 0.0 => misses += 1,
                        _ => {}
                    }
                    Some(json!({
                        "symbol": symbol,
                        "last_report": last.map(|r| r.date),
                        "eps_surprise": eps,
                        "revenue_surprise": revenue,
                        "next_report": next.map(|r| r.date),
                    }))
                })
                .collect();
            json!({"sector": sector.id, "beats": beats, "misses": misses, "members": members})
        })
        .collect();
    json!({"unit": "percent vs consensus", "sectors": sectors})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn sectors() -> Vec<SectorMembers> {
        vec![
            SectorMembers {
                id: "ai-chip".into(),
                name_zh: "AI 芯片".into(),
                name_en: "AI Chips".into(),
                symbols: vec!["NVDA".into(), "AMD".into()],
            },
            SectorMembers {
                id: "space".into(),
                name_zh: "航天".into(),
                name_en: "Space".into(),
                symbols: vec!["RKLB".into()],
            },
        ]
    }

    #[test]
    fn digest_keeps_dated_material_up_to_as_of_newest_first() {
        let ontology = json!({
            "schema_version": 4,
            "generated_at": "2026-09-06",
            "industries": [{
                "id": "ai-chip",
                "one_liner": "量的闸门",
                "members": [{"symbol": "NVDA", "role": "整机架算力"}],
                "brief": {"as_of": "2026-09-02", "question": "多久？", "body": "正文", "next": ["NVDA FY27Q3"]},
                "ai_valuation_logic": {"driver_chain": "①钱袋", "anti_pattern_short": "不要", "key_variables": [
                    {"name": "CoWoS", "why": "闸门", "as_of": "2026-06-15"},
                    {"name": "未来", "why": "x", "as_of": "2026-12-01"}
                ]},
                "upstream_signals": [{"symbol": "NVDA", "name": "英伟达", "relation": "peer_signal", "latest": "DC $89.0B", "latest_as_of": "2026-08-26"}],
                "core_watch": [{"what": "拆分表", "cadence": "季度", "as_of": "2026-08-26"}],
                "sources": [
                    {"date": "2026-06-02", "house": "TrendForce", "takeaway": "old"},
                    {"date": "2026-08-26", "house": "NVIDIA", "takeaway": "new"},
                    {"date": "2026-11-01", "house": "Future", "takeaway": "not yet"}
                ]
            }]
        });
        let digest = ontology_digest(&ontology, &sectors(), d("2026-10-23"));
        let chip = &digest["sectors"][0];
        assert_eq!(chip["members"][0]["role"], "整机架算力");
        assert_eq!(chip["members"][1]["role"], "");
        let research = &chip["research"];
        assert_eq!(research["key_variables"].as_array().unwrap().len(), 1);
        let sources = research["recent_sources"].as_array().unwrap();
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0]["takeaway"], "new");
        assert_eq!(research["newest_material"], "2026-09-02");
        assert_eq!(research["stale"], false);
        // A sector the ontology does not describe is still listed.
        assert!(digest["sectors"][1]["research"].is_null());
        // Long after the material, it is flagged stale.
        let later = ontology_digest(&ontology, &sectors(), d("2027-06-01"));
        assert_eq!(later["sectors"][0]["research"]["stale"], true);
    }

    #[test]
    fn series_statistics() {
        let closes: Vec<f64> = (0..300).map(|t| 100.0 * 1.001f64.powi(t)).collect();
        let st = series_stats(&closes);
        assert_eq!(st.ret_1m, Some(pct(1.001f64.powi(21) - 1.0)));
        assert_eq!(st.from_high, Some(0.0));
        assert!(st.vs_200d.unwrap() > 0.0);
        assert_eq!(st.vol_3m, Some(0.0));
        let short = series_stats(&closes[..30]);
        assert_eq!(short.ret_3m, None);
        assert_eq!(short.vs_200d, None);
        assert_eq!(series_stats(&[]), SeriesStats::default());
    }

    #[test]
    fn equal_weight_index_averages_daily_returns() {
        let a = [(d("2026-01-01"), 100.0), (d("2026-01-02"), 110.0)];
        let b = [(d("2026-01-01"), 50.0), (d("2026-01-02"), 50.0)];
        let index = equal_weight_index(&[&a, &b]);
        assert_eq!(index.len(), 2);
        assert!((index[1] - 1.05).abs() < 1e-12);
    }

    #[test]
    fn earnings_use_only_reports_up_to_as_of() {
        let report = |date: &str, eps: Option<f64>, est: f64| EarningsReport {
            symbol: "NVDA".into(),
            date: d(date),
            eps_actual: eps,
            eps_estimate: Some(est),
            revenue_actual: None,
            revenue_estimate: None,
        };
        let reports = HashMap::from([(
            "NVDA".to_string(),
            vec![
                report("2026-05-28", Some(0.9), 1.0),
                report("2026-08-26", Some(1.21), 1.10),
                report("2026-11-19", None, 1.30),
            ],
        )]);
        let summary = earnings_summary(&sectors(), &reports, d("2026-10-23"));
        let chip = &summary["sectors"][0];
        assert_eq!(chip["beats"], 1);
        assert_eq!(chip["members"][0]["last_report"], "2026-08-26");
        assert_eq!(chip["members"][0]["eps_surprise"], 10.0);
        assert_eq!(chip["members"][0]["next_report"], "2026-11-19");
        assert_eq!(chip["members"].as_array().unwrap().len(), 1);
        // Before the August report, the May miss is the latest.
        let summary = earnings_summary(&sectors(), &reports, d("2026-08-25"));
        assert_eq!(summary["sectors"][0]["misses"], 1);
    }
}
