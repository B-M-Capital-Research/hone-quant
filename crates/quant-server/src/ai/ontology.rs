//! honeclaw's ontology as honeclaw serves it: the base file plus the append-only edit log that
//! administrators write from honeclaw's industry-map page (`data/industry_map/edits.json`).
//!
//! The log is replayed in order with honeclaw's own rules (`hone_core::industry_map::apply`): an
//! edit that cannot apply (an unknown industry, member, signal or watch item, a duplicate, an
//! invalid date or field) is skipped, exactly as honeclaw skips it. Edits to the forward
//! valuation and its subtypes are not replayed: the briefing does not read those fields, and no
//! other edit depends on them.

use chrono::NaiveDate;
use serde_json::{Map, Value, json};

/// Upstream relations honeclaw accepts (`UPSTREAM_RELATIONS`).
const RELATIONS: [&str; 4] = [
    "demand_source",
    "capex_source",
    "supply_gate",
    "peer_signal",
];

/// Text fields `set_field` may change (`EDITABLE_FIELDS`); all but `one_liner` live in
/// `ai_valuation_logic`.
const EDITABLE_FIELDS: [&str; 6] = [
    "one_liner",
    "driver_chain",
    "multiple_anchor",
    "anti_pattern",
    "multiple_anchor_short",
    "anti_pattern_short",
];

/// A date as honeclaw writes it: `YYYY-MM-DD`, or `YYYY-MM` meaning the first of the month.
pub fn parse_as_of(text: &str) -> Option<NaiveDate> {
    let text = text.trim();
    if let Ok(date) = NaiveDate::parse_from_str(text, "%Y-%m-%d") {
        return Some(date);
    }
    let month = text.len() == 7
        && text.bytes().enumerate().all(|(i, b)| {
            if i == 4 {
                b == b'-'
            } else {
                b.is_ascii_digit()
            }
        });
    month
        .then(|| NaiveDate::parse_from_str(&format!("{text}-01"), "%Y-%m-%d").ok())
        .flatten()
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn list_mut<'a>(industry: &'a mut Value, key: &str) -> &'a mut Vec<Value> {
    if !industry.get(key).is_some_and(Value::is_array) {
        industry[key] = json!([]);
    }
    industry[key].as_array_mut().expect("just made an array")
}

/// Replays `edits` onto `ontology` and returns how many applied.
pub fn apply_edits(ontology: &mut Value, edits: &Value) -> usize {
    let Some(list) = edits.get("edits").and_then(Value::as_array) else {
        return 0;
    };
    list.iter()
        .filter(|edit| apply(ontology, edit).is_some())
        .count()
}

/// One edit; `None` when honeclaw would refuse it.
fn apply(map: &mut Value, edit: &Value) -> Option<()> {
    let id = text(edit, "industry").to_string();
    let op = edit.get("op")?;
    let root = text(map.get("root").unwrap_or(&Value::Null), "id").to_string();
    let industries = map.get_mut("industries")?.as_array_mut()?;
    match text(op, "kind") {
        "add_industry" => {
            let id = id.trim();
            let valid = !id.is_empty()
                && id
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
            if !valid || industries.iter().any(|i| text(i, "id") == id) {
                return None;
            }
            let new = op.get("industry")?;
            industries.push(json!({
                "id": id,
                "name": text(new, "name"),
                "parent": root,
                "one_liner": text(new, "one_liner"),
                "aliases": new.get("aliases").cloned().unwrap_or_else(|| json!([])),
                "ai_valuation_logic": {},
                "core_watch": [],
                "members": [],
                "sources": [],
                "upstream_signals": [],
                "valuation": {},
                "brief": null,
            }));
            return Some(());
        }
        "remove_industry" => {
            let before = industries.len();
            industries.retain(|i| text(i, "id") != id);
            return (industries.len() < before).then_some(());
        }
        _ => {}
    }
    let industry = industries.iter_mut().find(|i| text(i, "id") == id)?;
    match text(op, "kind") {
        "add_upstream_signal" => {
            let signal = op.get("signal")?;
            let signals = list_mut(industry, "upstream_signals");
            if !RELATIONS.contains(&text(signal, "relation"))
                || signals
                    .iter()
                    .any(|s| text(s, "symbol") == text(signal, "symbol"))
            {
                return None;
            }
            signals.push(signal.clone());
        }
        "remove_upstream_signal" => {
            let signals = list_mut(industry, "upstream_signals");
            let before = signals.len();
            signals.retain(|s| text(s, "symbol") != text(op, "symbol"));
            if signals.len() == before {
                return None;
            }
        }
        "set_upstream_latest" => {
            let signal = list_mut(industry, "upstream_signals")
                .iter_mut()
                .find(|s| text(s, "symbol") == text(op, "symbol"))?;
            signal["latest"] = json!(text(op, "latest").trim());
            signal["latest_as_of"] = json!(text(op, "as_of").trim());
        }
        "set_valuation_field"
        | "set_valuation_list"
        | "upsert_subtype"
        | "remove_subtype"
        | "set_member_subtype" => {
            // Valuation only: not in the briefing.
        }
        "set_field" => {
            let field = text(op, "field");
            if !EDITABLE_FIELDS.contains(&field) {
                return None;
            }
            let value = json!(text(op, "value"));
            if field == "one_liner" {
                industry["one_liner"] = value;
            } else {
                if !industry
                    .get("ai_valuation_logic")
                    .is_some_and(Value::is_object)
                {
                    industry["ai_valuation_logic"] = Value::Object(Map::new());
                }
                industry["ai_valuation_logic"][field] = value;
            }
        }
        "add_member" => {
            let member = op.get("member")?;
            let members = list_mut(industry, "members");
            if members
                .iter()
                .any(|m| text(m, "symbol") == text(member, "symbol"))
            {
                return None;
            }
            members.push(member.clone());
        }
        "remove_member" => {
            let members = list_mut(industry, "members");
            let before = members.len();
            members.retain(|m| text(m, "symbol") != text(op, "symbol"));
            if members.len() == before {
                return None;
            }
        }
        "set_member_role" => {
            let member = list_mut(industry, "members")
                .iter_mut()
                .find(|m| text(m, "symbol") == text(op, "symbol"))?;
            member["role"] = json!(text(op, "role"));
        }
        "add_source" => {
            let source = op.get("source")?;
            let url = text(source, "url");
            let sources = list_mut(industry, "sources");
            // A source folded back into the base file is not added twice.
            if url.is_empty() || !sources.iter().any(|s| text(s, "url") == url) {
                sources.push(source.clone());
            }
        }
        "remove_source" => {
            list_mut(industry, "sources").retain(|s| text(s, "url") != text(op, "url"));
        }
        "add_watch" => {
            let watch = op.get("watch")?;
            let as_of = text(watch, "as_of");
            let items = list_mut(industry, "core_watch");
            if items
                .iter()
                .any(|w| text(w, "what").trim() == text(watch, "what").trim())
                || (!as_of.is_empty() && parse_as_of(as_of).is_none())
            {
                return None;
            }
            items.push(watch.clone());
        }
        "set_watch" => {
            let watch = op.get("watch")?;
            let as_of = text(watch, "as_of");
            let items = list_mut(industry, "core_watch");
            let position = items
                .iter()
                .position(|w| text(w, "what").trim() == text(op, "what").trim())?;
            let duplicate = items.iter().enumerate().any(|(i, w)| {
                i != position && text(w, "what").trim() == text(watch, "what").trim()
            });
            if duplicate || (!as_of.is_empty() && parse_as_of(as_of).is_none()) {
                return None;
            }
            items[position] = watch.clone();
        }
        "remove_watch" => {
            list_mut(industry, "core_watch").retain(|w| text(w, "what") != text(op, "what"));
        }
        "set_brief" => {
            let brief = op.get("brief")?;
            if text(brief, "question").trim().is_empty()
                || parse_as_of(text(brief, "as_of")).is_none()
            {
                return None;
            }
            let next: Vec<&str> = brief
                .get("next")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .collect();
            industry["brief"] = json!({
                "question": text(brief, "question").trim(),
                "body": text(brief, "body").trim(),
                "next": next,
                "as_of": text(brief, "as_of").trim(),
            });
        }
        "clear_brief" => industry["brief"] = Value::Null,
        _ => return None,
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Value {
        json!({
            "root": {"id": "ai-datacenter"},
            "industries": [{
                "id": "power",
                "one_liner": "old",
                "ai_valuation_logic": {"driver_chain": "old chain"},
                "members": [{"symbol": "VST", "role": "old role"}],
                "sources": [{"url": "u1", "date": "2026-08-01"}],
                "core_watch": [{"what": "并网", "as_of": "2026-08"}],
                "upstream_signals": [{"symbol": "GEV", "relation": "supply_gate", "latest": "x", "latest_as_of": "2026-07-01"}],
                "brief": {"question": "q", "body": "b", "next": [], "as_of": "2026-08-27"}
            }]
        })
    }

    fn edit(industry: &str, op: Value) -> Value {
        json!({"at": "2026-10-07T00:00:00+08:00", "by": "x", "industry": industry, "op": op})
    }

    #[test]
    fn dates_take_day_or_month_precision() {
        assert_eq!(
            parse_as_of("2026-10-07"),
            NaiveDate::from_ymd_opt(2026, 10, 7)
        );
        assert_eq!(
            parse_as_of(" 2026-10 "),
            NaiveDate::from_ymd_opt(2026, 10, 1)
        );
        assert_eq!(parse_as_of("2026/10"), None);
        assert_eq!(parse_as_of(""), None);
    }

    #[test]
    fn edits_replay_in_order_and_refused_ones_are_skipped() {
        let mut map = base();
        let edits = json!({"schema_version": 1, "edits": [
            edit("power", json!({"kind": "set_brief", "brief": {"question": " 新问题 ", "body": "新正文", "next": [" a ", ""], "as_of": "2026-10-07"}})),
            edit("power", json!({"kind": "set_field", "field": "driver_chain", "value": "new chain"})),
            edit("power", json!({"kind": "set_field", "field": "members", "value": "refused"})),
            edit("power", json!({"kind": "set_member_role", "symbol": "VST", "role": "new role"})),
            edit("power", json!({"kind": "set_member_role", "symbol": "NOPE", "role": "refused"})),
            edit("power", json!({"kind": "add_source", "source": {"url": "u1", "date": "2026-10-01"}})),
            edit("power", json!({"kind": "add_source", "source": {"url": "u2", "date": "2026-10-02", "takeaway": "t"}})),
            edit("power", json!({"kind": "add_watch", "watch": {"what": "并网", "as_of": ""}})),
            edit("power", json!({"kind": "add_watch", "watch": {"what": "燃机", "as_of": "bad"}})),
            edit("power", json!({"kind": "add_watch", "watch": {"what": "燃机", "as_of": "2026-10"}})),
            edit("power", json!({"kind": "set_upstream_latest", "symbol": "GEV", "latest": " y ", "as_of": "2026-10-05"})),
            edit("power", json!({"kind": "add_upstream_signal", "signal": {"symbol": "ETN", "relation": "bogus"}})),
            edit("power", json!({"kind": "set_valuation_field", "field": "transmission", "value": "v"})),
            edit("nope", json!({"kind": "clear_brief"})),
            edit("space-2", json!({"kind": "add_industry", "industry": {"id": "space-2", "name": "太空 2"}})),
            edit("Bad_Id", json!({"kind": "add_industry", "industry": {"id": "Bad_Id", "name": "x"}})),
        ]});
        assert_eq!(apply_edits(&mut map, &edits), 9);
        let power = &map["industries"][0];
        assert_eq!(power["brief"]["question"], "新问题");
        assert_eq!(power["brief"]["next"], json!(["a"]));
        assert_eq!(power["brief"]["as_of"], "2026-10-07");
        assert_eq!(power["ai_valuation_logic"]["driver_chain"], "new chain");
        assert_eq!(power["members"][0]["role"], "new role");
        assert_eq!(power["sources"].as_array().unwrap().len(), 2);
        assert_eq!(power["core_watch"].as_array().unwrap().len(), 2);
        assert_eq!(power["upstream_signals"][0]["latest"], "y");
        assert_eq!(power["upstream_signals"][0]["latest_as_of"], "2026-10-05");
        assert_eq!(map["industries"][1]["parent"], "ai-datacenter");
        // A later edit can depend on an earlier one, and removals take effect.
        let more = json!({"edits": [
            edit("power", json!({"kind": "set_watch", "what": "燃机", "watch": {"what": "燃气轮机", "as_of": "2026-10-09"}})),
            edit("power", json!({"kind": "remove_watch", "what": "并网"})),
            edit("power", json!({"kind": "clear_brief"})),
            edit("space-2", json!({"kind": "remove_industry"})),
        ]});
        assert_eq!(apply_edits(&mut map, &more), 4);
        assert_eq!(
            map["industries"][0]["core_watch"],
            json!([{"what": "燃气轮机", "as_of": "2026-10-09"}])
        );
        assert!(map["industries"][0]["brief"].is_null());
        assert_eq!(map["industries"].as_array().unwrap().len(), 1);
        assert_eq!(apply_edits(&mut map, &json!({})), 0);
    }
}
