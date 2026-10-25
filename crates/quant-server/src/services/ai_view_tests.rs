//! The AI sector view end to end on the demo market, with a scripted model: the view is formed
//! from the briefing and validated, a shadow strategy records it without trading it, a live one
//! trades it, and the daily attempt cap holds.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use chrono::{Duration, NaiveDate, TimeZone, Utc};
use quant_core::calendar::MarketCalendar;
use quant_core::strategy::{AiMode, StrategyParams};
use serde_json::{Value, json};

use crate::ai::llm::{ChatModel, Completion, LlmError};
use crate::services::{ai_view, bootstrap, marketdata, planner};
use crate::store::{ai as ai_store, portfolios, strategy as strategy_store, trading};
use crate::testkit;

/// Answers every request with the same text and keeps the prompts it was given.
struct Scripted {
    answer: String,
    prompts: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl ChatModel for Scripted {
    fn model(&self) -> &str {
        "scripted"
    }

    async fn complete_json(&self, _system: &str, user: &str) -> Result<Completion, LlmError> {
        self.prompts.lock().unwrap().push(user.to_string());
        Ok(Completion {
            content: self.answer.clone(),
            model: "scripted".into(),
            usage: Some(json!({"total_tokens": 1})),
            finish_reason: Some("stop".into()),
            latency_ms: 5,
        })
    }
}

fn evidence(source: &str, date: &str) -> Value {
    json!({"source": source, "ref": "x", "date": date, "note_zh": "证据", "note_en": "evidence"})
}

fn answer() -> String {
    json!({
        "summary_zh": "电力交付瓶颈缓解，存储价格回落。",
        "summary_en": "Power delivery eases; memory prices soften.",
        "sectors": [
            {"sector": "power", "stance": 2, "confidence": 0.8, "rationale_zh": "并网提前",
             "evidence": [evidence("ontology", "2026-09-01"), evidence("earnings", "2026-08-01")]},
            {"sector": "storage", "stance": -2, "confidence": 0.8, "rationale_zh": "合约价下行",
             "evidence": [evidence("ontology", "2026-09-01"), evidence("market", "2026-10-01")]},
            // Price action alone: neutralised by validation.
            {"sector": "ai-chip", "stance": 1, "confidence": 0.7,
             "evidence": [evidence("market", "2026-10-01"), evidence("market", "2026-09-30")]}
        ]
    })
    .to_string()
}

fn weights(diagnostics: &Value) -> BTreeMap<String, f64> {
    diagnostics["targets"]["assets"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|a| a["weight"].as_f64().unwrap() > 0.0)
        .map(|a| {
            (
                a["symbol"].as_str().unwrap().to_string(),
                a["weight"].as_f64().unwrap(),
            )
        })
        .collect()
}

fn recorded(diagnostics: &Value, key: &str) -> BTreeMap<String, f64> {
    serde_json::from_value(diagnostics["ai"][key].clone()).unwrap()
}

#[tokio::test]
async fn views_are_formed_validated_recorded_in_shadow_and_traded_live() {
    // Monday 2026-10-05, 11:00 New York: the session is open.
    let now = Utc.with_ymd_and_hms(2026, 10, 5, 15, 0, 0).unwrap();
    let Some((_db, mut state)) = testkit::state_at(now).await else {
        eprintln!("skipped: HONE_QUANT_TEST_DATABASE_URL not set");
        return;
    };
    let ontology = std::env::temp_dir().join(format!("hq-ontology-{}.json", uuid::Uuid::new_v4()));
    std::fs::write(
        &ontology,
        json!({
            "schema_version": 4,
            "generated_at": "2026-09-06",
            "industries": [{
                "id": "power", "one_liner": "交付时点决定价值",
                "members": [{"symbol": "VST", "role": "电力运营"}],
                "brief": {"as_of": "2026-09-01", "question": "并网多快？", "body": "排队缩短", "next": []},
                "sources": [{"date": "2026-09-01", "house": "EIA", "takeaway": "并网排队缩短"}]
            }]
        })
        .to_string(),
    )
    .unwrap();
    let scripted = Arc::new(Scripted {
        answer: answer(),
        prompts: Mutex::new(Vec::new()),
    });
    {
        let state = Arc::get_mut(&mut state).expect("only reference");
        state.llm = Some(scripted.clone());
        Arc::get_mut(&mut state.config)
            .expect("only reference")
            .ai
            .ontology = ontology.display().to_string();
    }
    let today = MarketCalendar::local_date(now);
    bootstrap::run(&state).await.unwrap();
    marketdata::sync_daily(&state, 7, false).await.unwrap();
    marketdata::poll_quotes(&state).await.unwrap();
    let client = state.pool.get().await.unwrap();
    let main = portfolios::active_books(&client).await.unwrap()[0].clone();
    drop(client);
    assert!(
        !ai_view::needed(&state).await.unwrap(),
        "the default strategy does not consult it"
    );

    // Forming the view.
    let generated = ai_view::generate(&state, "test").await.unwrap();
    assert_eq!(generated.status, "ok", "{:?}", generated.error);
    let friday = NaiveDate::from_ymd_opt(2026, 10, 2).unwrap();
    assert_eq!(generated.as_of, friday);
    let prompt = scripted.prompts.lock().unwrap()[0].clone();
    assert!(prompt.contains("2026-10-02"));
    assert!(
        prompt.contains("并网排队缩短"),
        "the ontology digest is in the briefing"
    );
    assert!(
        prompt.contains("share_above_200d"),
        "so are the market statistics"
    );
    let client = state.pool.get().await.unwrap();
    let stored = ai_store::get(&client, generated.id).await.unwrap().unwrap();
    let view = stored.view().unwrap();
    let sector = |id: &str| view.sectors.iter().find(|s| s.sector == id).unwrap();
    assert_eq!(sector("power").stance, 2);
    assert_eq!(sector("storage").stance, -2);
    assert_eq!(sector("ai-chip").stance, 0);
    assert_eq!(sector("ai-chip").proposed_stance, Some(1));
    assert_eq!(sector("optical").flags, vec!["missing"]);
    assert!(stored.material.unwrap()["market"]["sectors"].is_array());

    // A shadow strategy trades without the view and records what it would have done.
    let mut params = StrategyParams::default();
    params.ai.mode = AiMode::Shadow;
    let shadow = strategy_store::insert_version(
        &client,
        "shadow",
        "sector_risk_budget",
        &params,
        "",
        "test",
    )
    .await
    .unwrap();
    strategy_store::activate(&client, main.account.id, shadow.id, "test", "")
        .await
        .unwrap();
    drop(client);
    assert!(ai_view::needed(&state).await.unwrap());
    let request = |slot: &str| planner::PlanRequest {
        slot: slot.into(),
        trade_date: today,
        deadline: now + Duration::hours(4),
        actor: "test".into(),
    };
    let plan = planner::generate(&state, main.portfolio.id, request("open"))
        .await
        .unwrap();
    let client = state.pool.get().await.unwrap();
    let diagnostics = trading::plan_diagnostics(&client, plan.plan_id)
        .await
        .unwrap();
    assert_eq!(diagnostics["ai"]["status"], "shadow");
    assert_eq!(diagnostics["ai"]["view_id"], generated.id);
    let traded = weights(&diagnostics);
    let without = recorded(&diagnostics, "weights_without");
    let with = recorded(&diagnostics, "weights_with");
    assert_eq!(traded.len(), without.len());
    for (symbol, w) in &traded {
        assert!(
            (w - without[symbol]).abs() < 1e-12,
            "{symbol} traded the engine's weight"
        );
    }
    let sector_weight = |key: &str, id: &str| {
        diagnostics["ai"]["sectors"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["sector"] == id)
            .unwrap()[key]
            .as_f64()
            .unwrap()
    };
    assert!(sector_weight("weight_with", "power") > sector_weight("weight_without", "power"));
    assert!(sector_weight("weight_with", "storage") < sector_weight("weight_without", "storage"));
    assert!(diagnostics["ai"]["active_share"].as_f64().unwrap() > 0.0);
    // The view never changes how much is invested.
    let invested = |w: &BTreeMap<String, f64>| w.values().sum::<f64>();
    assert!((invested(&with) - invested(&without)).abs() < 1e-9);

    // A live strategy trades with it.
    params.ai.mode = AiMode::Live;
    let live =
        strategy_store::insert_version(&client, "live", "sector_risk_budget", &params, "", "test")
            .await
            .unwrap();
    strategy_store::activate(&client, main.account.id, live.id, "test", "")
        .await
        .unwrap();
    drop(client);
    let plan = planner::generate(&state, main.portfolio.id, request("manual"))
        .await
        .unwrap();
    let client = state.pool.get().await.unwrap();
    let diagnostics = trading::plan_diagnostics(&client, plan.plan_id)
        .await
        .unwrap();
    assert_eq!(diagnostics["ai"]["status"], "live");
    let traded = weights(&diagnostics);
    let with = recorded(&diagnostics, "weights_with");
    for (symbol, w) in &traded {
        assert!(
            (w - with[symbol]).abs() < 1e-12,
            "{symbol} traded the view's weight"
        );
    }

    // Too old for the strategy's limit: the plan runs without it.
    let later = today + Duration::days(params.ai.max_age_days as i64 + 4);
    assert!(
        ai_view::usable(&client, &params.ai, later)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        ai_view::usable(&client, &params.ai, today)
            .await
            .unwrap()
            .is_some()
    );

    // Both plans are in the evaluation (no closes after them yet).
    let evaluation = ai_view::evaluate(&state, main.account.id).await.unwrap();
    assert_eq!(evaluation.plans, 2);
    assert_eq!(evaluation.days, 0);
    drop(client);

    // The daily attempt cap.
    for _ in 1..ai_view::MAX_ATTEMPTS_PER_DAY {
        ai_view::generate(&state, "test").await.unwrap();
    }
    assert!(ai_view::capped(&state).await.unwrap());
    assert!(ai_view::generate(&state, "test").await.is_err());
    std::fs::remove_file(ontology).ok();
}
