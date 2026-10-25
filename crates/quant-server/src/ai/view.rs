//! The AI sector view: the prompt, and the strict validation of the model's answer.
//!
//! The model proposes a stance per sector with dated evidence; nothing it says reaches the engine
//! unchecked. Validation is deterministic: unknown sectors are dropped, numbers are clamped,
//! evidence dated after `as_of` is discarded, and a non-zero stance that is not backed by at
//! least two pieces of evidence, one of them dated research from the ontology or an earnings
//! report, is set back to neutral. Every correction is recorded as a flag on the sector.

use std::collections::{BTreeMap, BTreeSet};

use chrono::NaiveDate;
use quant_core::strategy::SectorView;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::llm::truncate;
use super::ontology::parse_as_of;

/// Bumped whenever the prompt or the validation rules change; stored with every view.
pub const PROMPT_VERSION: &str = "sector-view/1";

/// Highest confidence a view may carry.
pub const MAX_CONFIDENCE: f64 = 0.9;
/// Evidence a non-zero stance needs, at least one of it dated ontology research or earnings.
pub const MIN_EVIDENCE: usize = 2;
const MAX_EVIDENCE: usize = 6;

const SOURCES: [&str; 3] = ["ontology", "earnings", "market"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    /// `ontology`, `earnings` or `market`.
    pub source: String,
    /// Where in the material, e.g. `ai-chip.upstream_signals.NVDA`.
    #[serde(rename = "ref")]
    pub reference: String,
    pub date: Option<NaiveDate>,
    pub note_zh: String,
    pub note_en: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectorOpinion {
    pub sector: String,
    /// −2 (lean away) … +2 (lean in), after validation.
    pub stance: i8,
    pub confidence: f64,
    pub rationale_zh: String,
    pub rationale_en: String,
    pub risks_zh: String,
    pub risks_en: String,
    pub evidence: Vec<Evidence>,
    /// The model's own stance when validation changed it.
    pub proposed_stance: Option<i8>,
    /// Corrections made by validation: `missing`, `stance_clamped`, `confidence_clamped`,
    /// `evidence_dropped`, `future_evidence_dropped`, `insufficient_evidence`.
    pub flags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewOutput {
    pub as_of: NaiveDate,
    pub summary_zh: String,
    pub summary_en: String,
    pub sectors: Vec<SectorOpinion>,
    /// Answer-level problems (sectors the universe does not have, duplicates).
    pub notes: Vec<String>,
}

impl ViewOutput {
    /// The engine's input: every sector's validated stance and confidence.
    pub fn sector_views(&self) -> BTreeMap<String, SectorView> {
        self.sectors
            .iter()
            .map(|s| {
                (
                    s.sector.clone(),
                    SectorView {
                        stance: s.stance,
                        confidence: s.confidence,
                    },
                )
            })
            .collect()
    }
}

pub fn system_prompt(max_tilt: f64) -> String {
    let up = 1.0 + max_tilt;
    format!(
        "你是 hone-quant 的板块配置分析员。hone-quant 是一个只做模拟盘的规则化组合：先按风险（板块等权指数波动率的倒数）给 AI 基础设施的各个板块分配预算，再在板块内按波动率倒数分配公司，市场宽度决定总仓位，其余留现金。\n\
\n\
你唯一的作用：对每个板块给出一个相对观点 stance（-2 到 +2 的整数）和置信度 confidence（0 到 {MAX_CONFIDENCE}）。引擎把它换算成板块得分的乘数，最多 ±{tilt:.0}%（stance +2、置信度 1 时为 ×{up:.2}），之后板块下限与上限、单一公司上限和波动率目标照常生效。你不能选股、不能改变总仓位、不能下单。\n\
\n\
规则：\n\
1. 默认 stance = 0。只有当材料中的证据表明未来 1–3 个月该板块的基本面相对其它板块明显更好或更差（需求、供给瓶颈、价格、指引、业绩相对一致预期）时，才给非零观点。\n\
2. 只使用本次提供的材料。不要使用材料之外的知识，也不要使用你对 as_of 之后发生之事的任何了解；材料里没有的事，当作不知道。\n\
3. 价格涨跌本身不是证据：在这个股票池里，动量和趋势信号经回测没有预测力。行情数据（market）只用来判断拥挤、风险和预期已反映多少，不能单独支持非零观点。\n\
4. 每个非零观点至少引用 {MIN_EVIDENCE} 条证据，其中至少 1 条来自 ontology（研究员维护的产业本体）或 earnings（实际业绩对一致预期），每条写明日期（不晚于 as_of）。\n\
5. 研究材料标记为 stale（距 as_of 超过 120 天）的板块，除非有新的 earnings 证据，否则给 0。\n\
6. 观点是相对的，通常有正有负；如果所有板块看起来一样好或一样差，就都给 0。\n\
7. 置信度 0.5 表示有一定把握，0.8 表示把握较大；|stance| = 2 只用于证据一致且新近的情形。\n\
8. rationale 说清楚为什么是这个板块、为什么是现在；risks 写出最可能让观点出错的事。中文简洁，英文为对应的翻译。\n\
\n\
只输出一个 JSON 对象，不要输出任何其它文字。",
        tilt = max_tilt * 100.0,
    )
}

pub fn user_prompt(as_of: NaiveDate, sector_ids: &[String], material: &Value) -> String {
    format!(
        "as_of（材料截止的最后一个收盘日）：{as_of}\n\
板块 id：{ids}\n\
\n\
材料（JSON）：\n{material}\n\
\n\
输出格式（每个板块 id 恰好出现一次）：\n\
{{\"as_of\": \"{as_of}\", \"summary_zh\": \"不超过 120 字：本次整体判断\", \"summary_en\": \"the same in English\", \"sectors\": [{{\"sector\": \"<id>\", \"stance\": 0, \"confidence\": 0.0, \"rationale_zh\": \"不超过 90 字\", \"rationale_en\": \"...\", \"risks_zh\": \"不超过 50 字\", \"risks_en\": \"...\", \"evidence\": [{{\"source\": \"ontology|earnings|market\", \"ref\": \"例如 ai-chip.upstream_signals.NVDA\", \"date\": \"YYYY-MM-DD\", \"note_zh\": \"不超过 50 字\", \"note_en\": \"...\"}}]}}]}}",
        ids = sector_ids.join(", "),
    )
}

/// The JSON object in an answer, tolerating code fences or stray text around it.
fn json_object(content: &str) -> Option<Value> {
    let start = content.find('{')?;
    let end = content.rfind('}')?;
    (start < end)
        .then(|| serde_json::from_str(&content[start..=end]).ok())
        .flatten()
}

fn string(value: &Value, key: &str, max: usize) -> String {
    truncate(
        value
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim(),
        max,
    )
}

fn number(value: &Value, key: &str) -> Option<f64> {
    match value.get(key)? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
    .filter(|x: &f64| x.is_finite())
}

fn neutral(sector: &str, flag: &str) -> SectorOpinion {
    SectorOpinion {
        sector: sector.to_string(),
        stance: 0,
        confidence: 0.0,
        rationale_zh: String::new(),
        rationale_en: String::new(),
        risks_zh: String::new(),
        risks_en: String::new(),
        evidence: Vec::new(),
        proposed_stance: None,
        flags: vec![flag.to_string()],
    }
}

fn opinion(item: &Value, sector: &str, as_of: NaiveDate) -> SectorOpinion {
    let mut flags = Vec::new();
    let raw_stance = number(item, "stance").unwrap_or(0.0);
    let mut stance = raw_stance.round().clamp(-2.0, 2.0) as i8;
    if raw_stance != f64::from(stance) {
        flags.push("stance_clamped".to_string());
    }
    let raw_confidence = number(item, "confidence").unwrap_or(0.0);
    let confidence = raw_confidence.clamp(0.0, MAX_CONFIDENCE);
    if raw_confidence != confidence {
        flags.push("confidence_clamped".to_string());
    }

    let mut evidence = Vec::new();
    let mut dropped = false;
    let mut future = false;
    for e in item
        .get("evidence")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let source = string(e, "source", 20).to_lowercase();
        if !SOURCES.contains(&source.as_str()) {
            dropped = true;
            continue;
        }
        let date = e.get("date").and_then(Value::as_str).and_then(parse_as_of);
        if date.is_some_and(|d| d > as_of) {
            future = true;
            continue;
        }
        if evidence.len() < MAX_EVIDENCE {
            evidence.push(Evidence {
                source,
                reference: string(e, "ref", 80),
                date,
                note_zh: string(e, "note_zh", 80),
                note_en: string(e, "note_en", 160),
            });
        }
    }
    if dropped {
        flags.push("evidence_dropped".to_string());
    }
    if future {
        flags.push("future_evidence_dropped".to_string());
    }

    let mut proposed_stance = None;
    let grounded = evidence
        .iter()
        .any(|e| e.source != "market" && e.date.is_some());
    if stance != 0 && (evidence.len() < MIN_EVIDENCE || !grounded) {
        proposed_stance = Some(stance);
        stance = 0;
        flags.push("insufficient_evidence".to_string());
    }
    SectorOpinion {
        sector: sector.to_string(),
        stance,
        confidence,
        rationale_zh: string(item, "rationale_zh", 160),
        rationale_en: string(item, "rationale_en", 320),
        risks_zh: string(item, "risks_zh", 100),
        risks_en: string(item, "risks_en", 200),
        evidence,
        proposed_stance,
        flags,
    }
}

/// Validates an answer against the universe's sectors. Fails only when there is no usable JSON
/// object with a `sectors` list at all.
pub fn parse_view(
    content: &str,
    sector_ids: &[String],
    as_of: NaiveDate,
) -> Result<ViewOutput, String> {
    let answer = json_object(content).ok_or("the answer is not a JSON object")?;
    let items = answer
        .get("sectors")
        .and_then(Value::as_array)
        .ok_or("the answer has no `sectors` list")?;
    let known: BTreeSet<&str> = sector_ids.iter().map(String::as_str).collect();
    let mut found: BTreeMap<String, SectorOpinion> = BTreeMap::new();
    let mut notes = Vec::new();
    for item in items {
        let id = string(item, "sector", 40);
        if !known.contains(id.as_str()) {
            notes.push(format!("unknown sector {id:?} ignored"));
            continue;
        }
        if found.contains_key(&id) {
            notes.push(format!("duplicate sector {id} ignored"));
            continue;
        }
        found.insert(id.clone(), opinion(item, &id, as_of));
    }
    let sectors = sector_ids
        .iter()
        .map(|id| found.remove(id).unwrap_or_else(|| neutral(id, "missing")))
        .collect();
    Ok(ViewOutput {
        as_of,
        summary_zh: string(&answer, "summary_zh", 240),
        summary_en: string(&answer, "summary_en", 480),
        sectors,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn ids() -> Vec<String> {
        ["ai-chip", "storage", "power"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    fn as_of() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 23).unwrap()
    }

    fn evidence(source: &str, date: &str) -> Value {
        json!({"source": source, "ref": "x", "date": date, "note_zh": "n", "note_en": "n"})
    }

    #[test]
    fn grounded_views_pass_and_ungrounded_ones_are_neutralised() {
        let answer = json!({
            "summary_zh": "总体", "summary_en": "overall",
            "sectors": [
                {"sector": "ai-chip", "stance": 2, "confidence": 0.8, "rationale_zh": "需求确认",
                 "evidence": [evidence("ontology", "2026-09-02"), evidence("earnings", "2026-08-26")]},
                // Price action only: not enough.
                {"sector": "storage", "stance": -1, "confidence": 0.6,
                 "evidence": [evidence("market", "2026-10-23"), evidence("market", "2026-10-22")]},
                {"sector": "space", "stance": 2, "confidence": 0.9},
                {"sector": "ai-chip", "stance": -2, "confidence": 0.9}
            ]
        });
        let view = parse_view(&format!("```json\n{answer}\n```"), &ids(), as_of()).unwrap();
        assert_eq!(view.sectors.len(), 3);
        let chip = &view.sectors[0];
        assert_eq!((chip.stance, chip.confidence), (2, 0.8));
        assert!(chip.flags.is_empty());
        let storage = &view.sectors[1];
        assert_eq!(storage.stance, 0);
        assert_eq!(storage.proposed_stance, Some(-1));
        assert_eq!(storage.flags, vec!["insufficient_evidence"]);
        let power = &view.sectors[2];
        assert_eq!(power.flags, vec!["missing"]);
        assert_eq!(view.notes.len(), 2);
        let views = view.sector_views();
        assert_eq!(views["ai-chip"].stance, 2);
        assert_eq!(views["storage"].stance, 0);
    }

    #[test]
    fn future_dated_evidence_does_not_count() {
        let answer = json!({"sectors": [
            {"sector": "power", "stance": 1, "confidence": 0.7,
             "evidence": [evidence("ontology", "2026-11-01"), evidence("earnings", "2026-12-01"), evidence("market", "2026-10-20")]}
        ]});
        let view = parse_view(&answer.to_string(), &ids(), as_of()).unwrap();
        let power = &view.sectors[2];
        assert_eq!(power.stance, 0);
        assert_eq!(power.evidence.len(), 1);
        assert!(power.flags.contains(&"future_evidence_dropped".to_string()));
        assert!(power.flags.contains(&"insufficient_evidence".to_string()));
    }

    #[test]
    fn numbers_are_clamped_and_sources_checked() {
        let answer = json!({"sectors": [
            {"sector": "ai-chip", "stance": "3", "confidence": 1.4,
             "evidence": [evidence("ontology", "2026-09-02"), evidence("blog", "2026-10-01"), evidence("earnings", "2026-08-26")]}
        ]});
        let view = parse_view(&answer.to_string(), &ids(), as_of()).unwrap();
        let chip = &view.sectors[0];
        assert_eq!(chip.stance, 2);
        assert_eq!(chip.confidence, MAX_CONFIDENCE);
        assert_eq!(chip.evidence.len(), 2);
        assert_eq!(
            chip.flags,
            vec!["stance_clamped", "confidence_clamped", "evidence_dropped"]
        );
    }

    #[test]
    fn unusable_answers_fail() {
        assert!(parse_view("no json here", &ids(), as_of()).is_err());
        assert!(parse_view("{\"summary\": 1}", &ids(), as_of()).is_err());
    }

    #[test]
    fn prompts_name_the_bounds_and_sectors() {
        let system = system_prompt(0.25);
        assert!(system.contains("±25%"));
        assert!(system.contains("×1.25"));
        let user = user_prompt(as_of(), &ids(), &json!({"a": 1}));
        assert!(user.contains("2026-10-23"));
        assert!(user.contains("ai-chip, storage, power"));
    }
}
