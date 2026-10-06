//! The automation loop.
//!
//! Every tick (15 s) the scheduler looks at the clock, the NYSE calendar and the database and
//! does whatever is due. It keeps no state that matters across restarts: plan slots are made
//! idempotent by a unique index, other jobs by `(job, run_key)` claims, so a restart in the
//! middle of the trading day simply picks up where things stand. Only one process runs the
//! loop: leadership is a PostgreSQL session advisory lock held on a dedicated connection.
//!
//! Daily timeline on a regular session (New York time; Singapore is +12h in summer):
//! - open −90 min: refresh daily history, corporate actions; apply splits/dividends;
//! - open −30 min: pre-open briefing (configurable);
//! - open +30 min: opening plan → review window → automatic execution (configurable);
//! - close −180 min: pre-close plan → review window → automatic execution;
//! - close −5 min: execution cut-off; unexecuted plans expire;
//! - close +15 min: refresh today's bars; closing NAV snapshot; daily summary at +20 min;
//! - last session of the week, close +30 min: weekly report.

use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use quant_core::calendar::MarketCalendar;
use quant_core::schedule::day_schedule;
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::db::lock_key;
use crate::notify::{self, Event};
use crate::services::reminders::{self, Schedule};
use crate::services::{broker, marketdata, planner, portfolio};
use crate::state::AppState;
use crate::store::settings::{
    self, AutomationMode, AutomationSettings, ExecutionSettings, RiskSettings,
};
use crate::store::strategy as strategy_store;
use crate::store::{market, system, trading};

const TICK: std::time::Duration = std::time::Duration::from_secs(15);

struct LoopState {
    last_quote_poll: Option<Instant>,
    last_housekeeping: Option<Instant>,
    last_risk_check: Option<Instant>,
    initial_sync_done: bool,
}

pub fn spawn(state: Arc<AppState>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let loop_state = Mutex::new(LoopState {
            last_quote_poll: None,
            last_housekeeping: None,
            last_risk_check: None,
            initial_sync_done: false,
        });
        let mut leader: Option<tokio_postgres::Client> = None;
        loop {
            if leader.as_ref().is_none_or(|c| c.is_closed()) {
                leader = acquire_leadership(&state).await;
                if leader.is_none() {
                    tracing::debug!("another hone-quant instance runs the scheduler");
                }
            }
            if leader.is_some() {
                let mut ls = loop_state.lock().await;
                if let Err(error) = tick(&state, &mut ls).await {
                    tracing::error!(error = %format!("{error:#}"), "scheduler tick failed");
                }
            }
            tokio::time::sleep(TICK).await;
        }
    })
}

async fn acquire_leadership(state: &AppState) -> Option<tokio_postgres::Client> {
    let (client, connection) = state
        .config
        .db
        .pg
        .connect(tokio_postgres::NoTls)
        .await
        .ok()?;
    tokio::spawn(async move {
        if let Err(error) = connection.await {
            tracing::warn!(%error, "scheduler leadership connection closed");
        }
    });
    let key = lock_key(&format!("scheduler:{}", state.config.db.schema));
    let row = client
        .query_one(
            "SELECT pg_try_advisory_lock(hashtextextended($1, 0))",
            &[&key],
        )
        .await
        .ok()?;
    let acquired: bool = row.get(0);
    if acquired {
        tracing::info!("scheduler leadership acquired");
        Some(client)
    } else {
        None
    }
}

/// Runs `work` once per `(job, key)`; a failed run may be retried after ten minutes.
async fn once<F, Fut>(state: &AppState, job: &str, key: &str, work: F) -> Result<bool>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<Value>>,
{
    let client = state.pool.get().await?;
    let mut claim = system::claim_job(&client, job, key).await?;
    if claim.is_none()
        && let Some(last) = system::job_runs(&client, Some(job), 5)
            .await?
            .into_iter()
            .find(|r| r.run_key == key)
    {
        let retry = last.status == "failed"
            && last
                .finished_at
                .is_some_and(|t| state.now() - t > Duration::minutes(10));
        if retry {
            claim = system::reclaim_failed_job(&client, job, key).await?;
        }
    }
    let Some(run_id) = claim else {
        return Ok(false);
    };
    drop(client);
    let result = work().await;
    let client = state.pool.get().await?;
    match result {
        Ok(detail) => {
            system::finish_job(&client, run_id, "succeeded", detail, None).await?;
            Ok(true)
        }
        Err(error) => {
            let message = format!("{error:#}");
            tracing::warn!(job, key, error = %message, "job failed");
            system::finish_job(&client, run_id, "failed", json!({}), Some(&message)).await?;
            Err(error)
        }
    }
}

async fn tick(state: &Arc<AppState>, ls: &mut LoopState) -> Result<()> {
    let now = state.now();
    let today = MarketCalendar::local_date(now);
    let session = state.calendar.session(today);

    // Housekeeping every 30 minutes.
    if ls
        .last_housekeeping
        .is_none_or(|t| t.elapsed() > std::time::Duration::from_secs(1800))
    {
        let client = state.pool.get().await?;
        system::purge_expired_sessions(&client).await?;
        system::fail_stale_jobs(&client).await?;
        ls.last_housekeeping = Some(Instant::now());
    }

    // History: fill gaps once per process start (cheap when already complete).
    if !ls.initial_sync_done {
        ls.initial_sync_done = true;
        let state2 = state.clone();
        tokio::spawn(async move {
            match marketdata::sync_daily(&state2, 7, false).await {
                Ok(report) => {
                    tracing::info!(
                        symbols = report.symbols,
                        bars = report.bars,
                        failed = report.failed.len(),
                        "initial history sync done"
                    );
                    if let Err(error) = marketdata::poll_quotes(&state2).await {
                        tracing::warn!(%error, "initial quote poll failed");
                    }
                }
                Err(error) => {
                    tracing::error!(error = %format!("{error:#}"), "initial history sync failed")
                }
            }
        });
    }

    let client = state.pool.get().await?;
    let execution: ExecutionSettings = settings::get(&client, settings::EXECUTION).await?;
    let automation: AutomationSettings = settings::get(&client, settings::AUTOMATION).await?;
    let schedule = settings::schedule(&client).await?;
    drop(client);
    let session_open = session.is_some_and(|s| s.contains(now));

    // Quotes: frequent during the session, occasionally otherwise (for display).
    let interval = if session_open {
        std::time::Duration::from_secs(execution.quote_poll_secs)
    } else {
        std::time::Duration::from_secs(900)
    };
    if ls.last_quote_poll.is_none_or(|t| t.elapsed() >= interval) {
        ls.last_quote_poll = Some(Instant::now());
        if let Err(error) = marketdata::poll_quotes(state).await {
            tracing::warn!(error = %format!("{error:#}"), "quote poll failed");
        }
    }

    if let Some(session) = session {
        let key = today.to_string();

        // Pre-open data refresh and corporate actions.
        if now >= session.open - Duration::minutes(90) {
            let s = state.clone();
            let _ = once(state, "preopen_sync", &key, || async move {
                let report = marketdata::sync_daily(&s, 10, false).await?;
                let actions = marketdata::sync_corporate_actions(&s, &[]).await.unwrap_or(0);
                let applied = portfolio::apply_corporate_actions(&s, today).await?;
                if !report.failed.is_empty() && report.failed.len() * 2 > report.symbols {
                    let _ = notify::notify(&s, Event::DataSyncFailed {
                        error: format!("{} of {} symbols failed to refresh", report.failed.len(), report.symbols),
                    })
                    .await;
                }
                Ok(json!({"bars": report.bars, "failed": report.failed, "actions": actions, "applied": applied}))
            })
            .await;
        }

        // Pre-open briefing.
        let client = state.pool.get().await?;
        let pre_open = reminders::builtin(&client, reminders::PRE_OPEN).await?;
        drop(client);
        if let Some(Schedule::PreOpen { minutes_before }) = pre_open
            && now >= session.open - Duration::minutes(minutes_before)
            && now < session.open
        {
            let s = state.clone();
            let slots: Vec<(String, DateTime<Utc>)> = day_schedule(&session, &schedule)
                .iter()
                .map(|slot| (slot.slot.as_str().to_string(), slot.generate_at))
                .collect();
            let mode = automation.effective_mode(now).as_str().to_string();
            let _ = once(state, "reminder_pre_open", &key, || async move {
                notify::notify(
                    &s,
                    Event::PreOpen {
                        trade_date: today,
                        open_at: session.open,
                        slots,
                        mode,
                    },
                )
                .await?;
                Ok(json!({}))
            })
            .await;
        }

        // Plan slots.
        let client = state.pool.get().await?;
        let account = trading::active_account(&client).await?;
        drop(client);
        if let Some(account) = account {
            for slot in day_schedule(&session, &schedule) {
                let slot_name = slot.slot.as_str();
                if now < slot.generate_at {
                    continue;
                }
                let client = state.pool.get().await?;
                let existing =
                    trading::plan_for_slot(&client, account.id, today, slot_name).await?;
                let skipped = strategy_store::is_slot_skipped(&client, today, slot_name).await?;
                drop(client);
                if existing.is_some() {
                    continue;
                }
                // Never generate a slot late: once its window has passed (for the opening plan,
                // three hours after the open) the slot is recorded as missed or failed.
                if now >= slot.generation_closes() {
                    let client = state.pool.get().await?;
                    let failed = system::job_runs(&client, Some("plan"), 20)
                        .await?
                        .into_iter()
                        .any(|r| {
                            r.run_key == format!("{today}:{slot_name}") && r.status == "failed"
                        });
                    drop(client);
                    let reason = if failed { "error" } else { "missed" };
                    planner::record_skipped_slot(
                        state,
                        today,
                        slot_name,
                        reason,
                        slot.execute_deadline,
                    )
                    .await?;
                    continue;
                }
                if skipped.is_some() {
                    planner::record_skipped_slot(
                        state,
                        today,
                        slot_name,
                        "operator",
                        slot.execute_deadline,
                    )
                    .await?;
                    continue;
                }
                if automation.effective_mode(now) == AutomationMode::Paused {
                    planner::record_skipped_slot(
                        state,
                        today,
                        slot_name,
                        "paused",
                        slot.execute_deadline,
                    )
                    .await?;
                    continue;
                }
                let s = state.clone();
                let run_key = format!("{today}:{slot_name}");
                let deadline = slot.execute_deadline;
                let _ = once(state, "plan", &run_key, || async move {
                    let generated = planner::generate(
                        &s,
                        planner::PlanRequest {
                            slot: slot_name.to_string(),
                            trade_date: today,
                            deadline,
                            actor: "scheduler".into(),
                        },
                    )
                    .await?;
                    Ok(serde_json::to_value(generated)?)
                })
                .await;
            }

            // Automatic execution of due plans.
            let client = state.pool.get().await?;
            let open_plans = trading::open_plans(&client, account.id).await?;
            let review = reminders::builtin(&client, reminders::PLAN_REVIEW).await?;
            drop(client);
            for plan in open_plans.iter().filter(|p| p.status == "pending") {
                if let Some(at) = plan.execute_after
                    && now >= at
                    && now < plan.deadline
                    && session_open
                {
                    if let Err(error) = broker::execute_plan(state, plan.id, "scheduler").await {
                        tracing::warn!(plan = plan.id, error = %format!("{error:#}"), "automatic execution failed");
                    }
                    continue;
                }
                // Review nudge before the automatic execution or the approval deadline.
                if let Some(Schedule::BeforeDeadline { minutes_before }) = &review {
                    let due = plan.execute_after.unwrap_or(plan.deadline);
                    let window = Duration::minutes(*minutes_before);
                    // Only when the review window is long enough for a nudge to be useful.
                    if now < due
                        && now >= due - window
                        && (due - plan.generated_at) > Duration::minutes(5)
                    {
                        let s = state.clone();
                        let (plan_id, slot, automatic) =
                            (plan.id, plan.slot.clone(), plan.execute_after.is_some());
                        let _ = once(
                            state,
                            "reminder_plan_review",
                            &plan.id.to_string(),
                            || async move {
                                notify::notify(
                                    &s,
                                    Event::PlanReview {
                                        plan_id,
                                        slot,
                                        due,
                                        automatic,
                                    },
                                )
                                .await?;
                                Ok(json!({}))
                            },
                        )
                        .await;
                    }
                }
            }
            broker::expire_overdue(state).await?;

            // Risk checks every five minutes during the session.
            if session_open
                && ls
                    .last_risk_check
                    .is_none_or(|t| t.elapsed() > std::time::Duration::from_secs(300))
            {
                ls.last_risk_check = Some(Instant::now());
                risk_checks(state, today).await?;
            }

            // After the close: today's bars, closing snapshot, daily summary.
            if now >= session.close + Duration::minutes(15) {
                let s = state.clone();
                let _ = once(state, "postclose_sync", &key, || async move {
                    let report = marketdata::sync_daily(&s, 5, false).await?;
                    Ok(json!({"bars": report.bars, "failed": report.failed}))
                })
                .await;
                let s = state.clone();
                let snapped = once(state, "eod_snapshot", &key, || async move {
                    let nav = portfolio::snapshot_eod(&s, today).await?;
                    Ok(json!({"nav": nav}))
                })
                .await
                .unwrap_or(false);
                if snapped {
                    state.emit(crate::state::ServerEvent::Account {
                        reason: "eod".into(),
                    });
                }
            }
            let client = state.pool.get().await?;
            let daily = reminders::builtin(&client, reminders::DAILY_SUMMARY).await?;
            let weekly = reminders::builtin(&client, reminders::WEEKLY_REPORT).await?;
            drop(client);
            if let Some(Schedule::PostClose { minutes_after }) = daily
                && now >= session.close + Duration::minutes(minutes_after.max(16))
            {
                let s = state.clone();
                let _ = once(state, "reminder_daily_summary", &key, || async move {
                    daily_summary(&s, today).await?;
                    Ok(json!({}))
                })
                .await;
            }
            let last_of_week =
                state.calendar.next_trading_day(today).iso_week() != today.iso_week();
            if let Some(Schedule::WeekClose { minutes_after }) = weekly
                && last_of_week
                && now >= session.close + Duration::minutes(minutes_after.max(16))
            {
                let s = state.clone();
                let _ = once(state, "reminder_weekly_report", &key, || async move {
                    weekly_report(&s, today).await?;
                    Ok(json!({}))
                })
                .await;
            }
        }
    }

    // Custom reminders.
    fire_custom_reminders(state, now).await?;
    // Deliver what quiet hours held back, once they are over.
    notify::flush_deferred(state).await?;
    Ok(())
}

async fn risk_checks(state: &Arc<AppState>, today: NaiveDate) -> Result<()> {
    let client = state.pool.get().await?;
    let risk: RiskSettings = settings::get(&client, settings::RISK).await?;
    let valuation = portfolio::valuation(state, &client).await?;
    let quotes = market::quotes(&client).await?;
    drop(client);
    let key = today.to_string();
    if valuation.drawdown <= -risk.drawdown_alert {
        let s = state.clone();
        let drawdown = valuation.drawdown;
        let threshold = risk.drawdown_alert;
        let _ = once(state, "risk_drawdown", &key, || async move {
            notify::notify(
                &s,
                Event::DrawdownAlert {
                    drawdown,
                    threshold,
                },
            )
            .await?;
            Ok(json!({"drawdown": drawdown}))
        })
        .await;
    }
    if let Some(day) = valuation.day_return
        && day <= -risk.daily_loss_alert
    {
        let s = state.clone();
        let threshold = risk.daily_loss_alert;
        let _ = once(state, "risk_daily_loss", &key, || async move {
            notify::notify(
                &s,
                Event::DailyLossAlert {
                    loss: day,
                    threshold,
                },
            )
            .await?;
            Ok(json!({"loss": day}))
        })
        .await;
    }
    let newest = quotes.values().map(|q| q.fetched_at).max();
    if let Some(newest) = newest {
        let minutes = (state.now() - newest).num_minutes();
        if minutes >= 10 {
            let s = state.clone();
            let hour_key = format!("{}", state.now().format("%Y-%m-%dT%H"));
            let _ = once(state, "data_stale", &hour_key, || async move {
                notify::notify(&s, Event::DataStale { minutes }).await?;
                Ok(json!({"minutes": minutes}))
            })
            .await;
        }
    }
    Ok(())
}

async fn daily_summary(state: &Arc<AppState>, date: NaiveDate) -> Result<()> {
    let client = state.pool.get().await?;
    let valuation = portfolio::valuation(state, &client).await?;
    let start = date.and_hms_opt(0, 0, 0).expect("midnight").and_utc() - Duration::hours(12);
    let (fills, _) = trading::list_fills(
        &client,
        valuation.account.id,
        &trading::FillFilter {
            symbol: None,
            from: Some(start),
            to: None,
            plan_id: None,
            limit: 1000,
            offset: 0,
        },
    )
    .await?;
    drop(client);
    let mut movers: Vec<(String, f64)> = valuation
        .positions
        .iter()
        .filter_map(|p| p.day_change_pct.map(|c| (p.symbol.clone(), c)))
        .collect();
    movers.sort_by(|a, b| b.1.total_cmp(&a.1));
    notify::notify(
        state,
        Event::DailySummary {
            date,
            nav: valuation.nav,
            day_pnl: valuation.day_pnl.unwrap_or(0.0),
            day_return: valuation.day_return.unwrap_or(0.0),
            total_return: valuation.total_return,
            trades: fills.len(),
            best: movers.first().cloned(),
            worst: movers.last().cloned(),
        },
    )
    .await?;
    Ok(())
}

async fn weekly_report(state: &Arc<AppState>, date: NaiveDate) -> Result<()> {
    let perf = portfolio::performance(state, None).await?;
    let week_start = date - Duration::days(date.weekday().num_days_from_monday() as i64 + 1);
    let base = perf
        .dates
        .iter()
        .zip(&perf.nav)
        .rev()
        .find(|(d, _)| **d <= week_start)
        .map(|(_, v)| *v)
        .or_else(|| perf.nav.first().copied())
        .unwrap_or(0.0);
    let last = perf.nav.last().copied().unwrap_or(0.0);
    let client = state.pool.get().await?;
    let account = trading::require_active_account(&client).await?;
    let from = (week_start + Duration::days(1))
        .and_hms_opt(0, 0, 0)
        .expect("midnight")
        .and_utc();
    let (_, trades) = trading::list_fills(
        &client,
        account.id,
        &trading::FillFilter {
            symbol: None,
            from: Some(from),
            to: None,
            plan_id: None,
            limit: 1,
            offset: 0,
        },
    )
    .await?;
    drop(client);
    notify::notify(
        state,
        Event::WeeklyReport {
            week_end: date,
            week_return: if base > 0.0 { last / base - 1.0 } else { 0.0 },
            total_return: perf.metrics.total_return,
            max_drawdown: perf.metrics.max_drawdown,
            trades: trades as usize,
            nav: last,
        },
    )
    .await?;
    Ok(())
}

async fn fire_custom_reminders(state: &Arc<AppState>, now: DateTime<Utc>) -> Result<()> {
    let client = state.pool.get().await?;
    let due: Vec<_> = system::reminders(&client)
        .await?
        .into_iter()
        .filter(|r| r.kind == "custom" && r.enabled && r.next_fire_at.is_some_and(|t| t <= now))
        .collect();
    drop(client);
    for reminder in due {
        let schedule: Option<Schedule> = serde_json::from_value(reminder.schedule.clone()).ok();
        let next = schedule
            .as_ref()
            .and_then(|s| reminders::next_fire(s, now, &state.calendar));
        let client = state.pool.get().await?;
        system::mark_reminder_fired(&client, reminder.id, now, next).await?;
        drop(client);
        notify::notify(
            state,
            Event::Reminder {
                title: reminder.title.clone(),
                note: reminder.note.clone(),
            },
        )
        .await?;
    }
    Ok(())
}
