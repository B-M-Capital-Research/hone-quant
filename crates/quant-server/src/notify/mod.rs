//! Notifications.
//!
//! Every notable event becomes one row in the in-app inbox (rendered once in Chinese and
//! English), is pushed to open browsers over SSE, and — subject to the operator's routing,
//! minimum severity and quiet hours — is delivered to the configured outbound channels in the
//! configured language. Events held back by quiet hours are sent as one digest when the quiet
//! window ends; critical events can bypass quiet hours.
//!
//! The US session runs overnight in Singapore (21:30–04:00 SGT in summer), so every time in a
//! message is shown in the operator's display zone *and* in New York time.

pub mod channels;

use anyhow::Result;
use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::Tz;
use quant_core::strategy::version_display_names;
use serde_json::{Value, json};

use crate::state::{AppState, ServerEvent};
use crate::store::settings::{self, ChannelMap, DisplaySettings, Lang, NotificationSettings};
use crate::store::system::{self, NotificationRow};
use channels::{ChannelConfig, OutboundMessage, Severity};

fn severity_rank(severity: Severity) -> u8 {
    match severity {
        Severity::Info | Severity::Success => 0,
        Severity::Warning => 1,
        Severity::Critical => 2,
    }
}

pub fn parse_severity(value: &str) -> Severity {
    match value {
        "success" => Severity::Success,
        "warning" => Severity::Warning,
        "critical" => Severity::Critical,
        _ => Severity::Info,
    }
}

pub fn money(value: f64) -> String {
    let negative = value < 0.0;
    let cents = (value.abs() * 100.0).round() as u128;
    let (whole, frac) = (cents / 100, cents % 100);
    let digits = whole.to_string();
    let mut grouped = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    format!("{}${grouped}.{frac:02}", if negative { "-" } else { "" })
}

pub fn pct(value: f64) -> String {
    format!(
        "{}{:.2}%",
        if value > 0.0 { "+" } else { "" },
        value * 100.0
    )
}

fn zone(display: &DisplaySettings) -> Tz {
    display
        .timezone
        .parse()
        .unwrap_or(chrono_tz::Asia::Singapore)
}

fn zone_label(tz: Tz) -> &'static str {
    match tz {
        chrono_tz::Asia::Singapore => "SGT",
        chrono_tz::Asia::Shanghai | chrono_tz::Asia::Hong_Kong | chrono_tz::Asia::Taipei => "CST",
        chrono_tz::Asia::Tokyo => "JST",
        _ => "local",
    }
}

/// "22:00 SGT (10:00 ET)", or with the date when it differs from New York's.
pub fn dual_time(at: DateTime<Utc>, display: &DisplaySettings) -> String {
    let tz = zone(display);
    let local = at.with_timezone(&tz);
    let ny = at.with_timezone(&chrono_tz::America::New_York);
    if local.date_naive() == ny.date_naive() {
        format!(
            "{} {} ({} ET)",
            local.format("%H:%M"),
            zone_label(tz),
            ny.format("%H:%M")
        )
    } else {
        format!(
            "{} {} ({} ET)",
            local.format("%m-%d %H:%M"),
            zone_label(tz),
            ny.format("%m-%d %H:%M")
        )
    }
}

fn slot_name(slot: &str, lang: Lang) -> &'static str {
    match (slot, lang) {
        ("open", Lang::Zh) => "开盘计划",
        ("open", Lang::En) => "opening plan",
        ("close", Lang::Zh) => "收盘前计划",
        ("close", Lang::En) => "pre-close plan",
        (_, Lang::Zh) => "手动计划",
        (_, Lang::En) => "manual plan",
    }
}

/// Everything hone-quant notifies about.
#[derive(Debug, Clone)]
pub enum Event {
    PlanGenerated {
        plan_id: i64,
        trade_date: NaiveDate,
        slot: String,
        orders: usize,
        buys: usize,
        sells: usize,
        turnover: f64,
        execute_after: Option<DateTime<Utc>>,
        deadline: DateTime<Utc>,
        strategy: String,
        preset_id: String,
    },
    PlanNoAction {
        plan_id: i64,
        trade_date: NaiveDate,
        slot: String,
    },
    PlanExecuted {
        plan_id: i64,
        trade_date: NaiveDate,
        slot: String,
        filled: usize,
        rejected: usize,
        bought: f64,
        sold: f64,
        costs: f64,
    },
    PlanFailed {
        plan_id: i64,
        slot: String,
        error: String,
    },
    PlanCancelled {
        plan_id: i64,
        slot: String,
        actor: String,
        reason: String,
    },
    PlanExpired {
        plan_id: i64,
        slot: String,
    },
    SlotSkipped {
        trade_date: NaiveDate,
        slot: String,
        /// `paused`, `operator` or `missed`.
        reason: String,
    },
    SlotsCancelled {
        trade_date: NaiveDate,
        slots: Vec<String>,
        actor: String,
        reason: String,
    },
    StrategyActivated {
        version_id: i64,
        name: String,
        preset_id: String,
        actor: String,
    },
    AutomationChanged {
        mode: String,
        paused_until: Option<DateTime<Utc>>,
        actor: String,
    },
    DrawdownAlert {
        drawdown: f64,
        threshold: f64,
    },
    DailyLossAlert {
        loss: f64,
        threshold: f64,
    },
    DataStale {
        minutes: i64,
    },
    DataSyncFailed {
        error: String,
    },
    CorporateAction {
        symbol: String,
        kind: String,
        detail: String,
    },
    DailySummary {
        date: NaiveDate,
        nav: f64,
        day_pnl: f64,
        day_return: f64,
        total_return: f64,
        trades: usize,
        best: Option<(String, f64)>,
        worst: Option<(String, f64)>,
    },
    WeeklyReport {
        week_end: NaiveDate,
        week_return: f64,
        total_return: f64,
        max_drawdown: f64,
        trades: usize,
        nav: f64,
    },
    PreOpen {
        trade_date: NaiveDate,
        open_at: DateTime<Utc>,
        slots: Vec<(String, DateTime<Utc>)>,
        mode: String,
    },
    Reminder {
        title: String,
        note: String,
    },
    PlanReview {
        plan_id: i64,
        slot: String,
        due: DateTime<Utc>,
        automatic: bool,
    },
    UniverseChanged {
        added: Vec<String>,
        removed: Vec<String>,
    },
    AccountReset {
        initial_cash: f64,
        actor: String,
    },
    Test,
}

pub struct Rendered {
    pub kind: &'static str,
    pub category: &'static str,
    pub severity: Severity,
    pub title_zh: String,
    pub title_en: String,
    pub body_zh: String,
    pub body_en: String,
    pub params: Value,
    pub link: Option<String>,
}

fn mode_name(mode: &str, lang: Lang) -> &'static str {
    match (mode, lang) {
        ("auto", Lang::Zh) => "自动执行",
        ("auto", Lang::En) => "automatic",
        ("approval", Lang::Zh) => "需人工确认",
        ("approval", Lang::En) => "approval required",
        (_, Lang::Zh) => "已暂停",
        (_, Lang::En) => "paused",
    }
}

pub fn render(event: &Event, display: &DisplaySettings) -> Rendered {
    use Lang::{En, Zh};
    match event {
        Event::PlanGenerated {
            plan_id,
            trade_date,
            slot,
            orders,
            buys,
            sells,
            turnover,
            execute_after,
            deadline,
            strategy,
            preset_id,
        } => {
            let (strategy_zh, strategy_en) = version_display_names(strategy, preset_id);
            let when_zh = match execute_after {
                Some(at) => format!(
                    "将于 {} 自动执行，执行前可在页面取消或调整。",
                    dual_time(*at, display)
                ),
                None => format!("等待人工确认，截止 {}。", dual_time(*deadline, display)),
            };
            let when_en = match execute_after {
                Some(at) => format!(
                    "Executes automatically at {}; cancel or adjust it before then.",
                    dual_time(*at, display)
                ),
                None => format!(
                    "Waiting for approval until {}.",
                    dual_time(*deadline, display)
                ),
            };
            Rendered {
                kind: "plan_generated",
                category: "plan",
                severity: Severity::Info,
                title_zh: format!("{trade_date} {}：{orders} 笔调仓", slot_name(slot, Zh)),
                title_en: format!("{trade_date} {}: {orders} orders", slot_name(slot, En)),
                body_zh: format!(
                    "策略「{strategy_zh}」生成 {buys} 笔买入、{sells} 笔卖出，单边换手 {:.1}%。\n{when_zh}",
                    turnover * 100.0
                ),
                body_en: format!(
                    "Strategy \"{strategy_en}\" proposes {buys} buys and {sells} sells, one-way turnover {:.1}%.\n{when_en}",
                    turnover * 100.0
                ),
                params: json!({"plan_id": plan_id, "orders": orders, "turnover": turnover}),
                link: Some(format!("/plans/{plan_id}")),
            }
        }
        Event::PlanNoAction {
            plan_id,
            trade_date,
            slot,
        } => Rendered {
            kind: "plan_no_action",
            category: "plan",
            severity: Severity::Info,
            title_zh: format!("{trade_date} {}：无需调仓", slot_name(slot, Zh)),
            title_en: format!("{trade_date} {}: no trades needed", slot_name(slot, En)),
            body_zh: "所有持仓都在再平衡容忍带内。".into(),
            body_en: "Every position is inside its rebalancing band.".into(),
            params: json!({"plan_id": plan_id}),
            link: Some(format!("/plans/{plan_id}")),
        },
        Event::PlanExecuted {
            plan_id,
            trade_date,
            slot,
            filled,
            rejected,
            bought,
            sold,
            costs,
        } => Rendered {
            kind: "plan_executed",
            category: "execution",
            severity: if *rejected > 0 {
                Severity::Warning
            } else {
                Severity::Success
            },
            title_zh: format!(
                "{trade_date} {}已执行：成交 {filled} 笔",
                slot_name(slot, Zh)
            ),
            title_en: format!(
                "{trade_date} {} executed: {filled} fills",
                slot_name(slot, En)
            ),
            body_zh: format!(
                "买入 {}，卖出 {}，交易成本 {}。{}",
                money(*bought),
                money(*sold),
                money(*costs),
                if *rejected > 0 {
                    format!("{rejected} 笔未成交，请查看原因。")
                } else {
                    String::new()
                }
            ),
            body_en: format!(
                "Bought {}, sold {}, trading costs {}.{}",
                money(*bought),
                money(*sold),
                money(*costs),
                if *rejected > 0 {
                    format!(" {rejected} orders were not filled — see reasons.")
                } else {
                    String::new()
                }
            ),
            params: json!({"plan_id": plan_id, "filled": filled, "rejected": rejected}),
            link: Some(format!("/plans/{plan_id}")),
        },
        Event::PlanFailed {
            plan_id,
            slot,
            error,
        } => Rendered {
            kind: "plan_failed",
            category: "execution",
            severity: Severity::Critical,
            title_zh: format!("{}失败", slot_name(slot, Zh)),
            title_en: format!("{} failed", capitalize(slot_name(slot, En))),
            body_zh: format!("没有任何成交被记账。错误：{error}"),
            body_en: format!("No fills were booked. Error: {error}"),
            params: json!({"plan_id": plan_id, "error": error}),
            link: Some(format!("/plans/{plan_id}")),
        },
        Event::PlanCancelled {
            plan_id,
            slot,
            actor,
            reason,
        } => Rendered {
            kind: "plan_cancelled",
            category: "plan",
            severity: Severity::Info,
            title_zh: format!("{}已取消", slot_name(slot, Zh)),
            title_en: format!("{} cancelled", capitalize(slot_name(slot, En))),
            body_zh: format!("{actor} 取消了计划。{}", reason_text(reason, Zh)),
            body_en: format!("{actor} cancelled the plan.{}", reason_text(reason, En)),
            params: json!({"plan_id": plan_id, "actor": actor}),
            link: Some(format!("/plans/{plan_id}")),
        },
        Event::PlanExpired { plan_id, slot } => Rendered {
            kind: "plan_expired",
            category: "plan",
            severity: Severity::Warning,
            title_zh: format!("{}已过期未执行", slot_name(slot, Zh)),
            title_en: format!("{} expired unexecuted", capitalize(slot_name(slot, En))),
            body_zh: "计划在截止时间前没有获得确认，未下任何订单。".into(),
            body_en: "The plan was not approved before its deadline; no orders were placed.".into(),
            params: json!({"plan_id": plan_id}),
            link: Some(format!("/plans/{plan_id}")),
        },
        Event::SlotSkipped {
            trade_date,
            slot,
            reason,
        } => {
            let (zh, en, severity) = match reason.as_str() {
                "paused" => (
                    "自动交易已暂停，本时段未生成计划。",
                    "Automation is paused; no plan was generated.",
                    Severity::Info,
                ),
                "operator" => (
                    "该时段已被手动取消。",
                    "This slot was cancelled by an operator.",
                    Severity::Info,
                ),
                "error" => (
                    "计划生成失败（已重试），本时段未交易。详情见任务记录。",
                    "Plan generation failed (after retries); nothing was traded in this slot. See the job log.",
                    Severity::Critical,
                ),
                _ => (
                    "服务在计划窗口内不可用，本时段错过。",
                    "The service was unavailable during the plan window; this slot was missed.",
                    Severity::Warning,
                ),
            };
            Rendered {
                kind: "slot_skipped",
                category: "plan",
                severity,
                title_zh: format!("{trade_date} {}未执行", slot_name(slot, Zh)),
                title_en: format!("{trade_date} {} skipped", slot_name(slot, En)),
                body_zh: zh.into(),
                body_en: en.into(),
                params: json!({"slot": slot, "reason": reason}),
                link: Some("/plans".into()),
            }
        }
        Event::SlotsCancelled {
            trade_date,
            slots,
            actor,
            reason,
        } => {
            let zh_slots: Vec<&str> = slots.iter().map(|s| slot_name(s, Zh)).collect();
            let en_slots: Vec<&str> = slots.iter().map(|s| slot_name(s, En)).collect();
            Rendered {
                kind: "slots_cancelled",
                category: "plan",
                severity: Severity::Warning,
                title_zh: format!("{trade_date} 的交易计划已取消"),
                title_en: format!("Trading plans for {trade_date} cancelled"),
                body_zh: format!(
                    "{actor} 取消了：{}。{}",
                    zh_slots.join("、"),
                    reason_text(reason, Zh)
                ),
                body_en: format!(
                    "{actor} cancelled: {}.{}",
                    en_slots.join(", "),
                    reason_text(reason, En)
                ),
                params: json!({"slots": slots, "actor": actor}),
                link: Some("/".into()),
            }
        }
        Event::StrategyActivated {
            version_id,
            name,
            preset_id,
            actor,
        } => {
            let (name_zh, name_en) = version_display_names(name, preset_id);
            Rendered {
                kind: "strategy_activated",
                category: "system",
                severity: Severity::Info,
                title_zh: format!("策略已切换为「{name_zh}」"),
                title_en: format!("Strategy switched to \"{name_en}\""),
                body_zh: format!("{actor} 启用了版本 #{version_id}，从下一个计划开始生效。"),
                body_en: format!(
                    "{actor} activated version #{version_id}; it applies from the next plan."
                ),
                params: json!({"version_id": version_id}),
                link: Some("/strategy".into()),
            }
        }
        Event::AutomationChanged {
            mode,
            paused_until,
            actor,
        } => {
            let until_zh = paused_until
                .map(|t| format!("，暂停至 {}", dual_time(t, display)))
                .unwrap_or_default();
            let until_en = paused_until
                .map(|t| format!(", paused until {}", dual_time(t, display)))
                .unwrap_or_default();
            Rendered {
                kind: "automation_changed",
                category: "system",
                severity: Severity::Warning,
                title_zh: format!("自动交易模式：{}", mode_name(mode, Zh)),
                title_en: format!("Automation mode: {}", mode_name(mode, En)),
                body_zh: format!("由 {actor} 修改{until_zh}。"),
                body_en: format!("Changed by {actor}{until_en}."),
                params: json!({"mode": mode}),
                link: Some("/settings".into()),
            }
        }
        Event::DrawdownAlert {
            drawdown,
            threshold,
        } => Rendered {
            kind: "drawdown_alert",
            category: "risk",
            severity: Severity::Critical,
            title_zh: format!("组合回撤 {}", pct(*drawdown)),
            title_en: format!("Portfolio drawdown {}", pct(*drawdown)),
            body_zh: format!("净值较历史高点回撤已超过预警线 {}。", pct(-threshold)),
            body_en: format!(
                "NAV is below its peak by more than the {} alert threshold.",
                pct(-threshold)
            ),
            params: json!({"drawdown": drawdown}),
            link: Some("/performance".into()),
        },
        Event::DailyLossAlert { loss, threshold } => Rendered {
            kind: "daily_loss_alert",
            category: "risk",
            severity: Severity::Warning,
            title_zh: format!("今日亏损 {}", pct(*loss)),
            title_en: format!("Down {} today", pct(*loss)),
            body_zh: format!("当日净值跌幅超过预警线 {}。", pct(-threshold)),
            body_en: format!(
                "Today's NAV decline exceeds the {} alert threshold.",
                pct(-threshold)
            ),
            params: json!({"loss": loss}),
            link: Some("/".into()),
        },
        Event::DataStale { minutes } => Rendered {
            kind: "data_stale",
            category: "data",
            severity: Severity::Warning,
            title_zh: "行情数据未更新".into(),
            title_en: "Market data is stale".into(),
            body_zh: format!("盘中已有 {minutes} 分钟没有拿到新的报价。计划会冻结报价过旧的标的。"),
            body_en: format!(
                "No fresh quotes for {minutes} minutes during the session. Plans freeze names with stale quotes."
            ),
            params: json!({"minutes": minutes}),
            link: Some("/settings/data".into()),
        },
        Event::DataSyncFailed { error } => Rendered {
            kind: "data_sync_failed",
            category: "data",
            severity: Severity::Warning,
            title_zh: "行情同步失败".into(),
            title_en: "Market data sync failed".into(),
            body_zh: error.clone(),
            body_en: error.clone(),
            params: json!({}),
            link: Some("/settings/data".into()),
        },
        Event::CorporateAction {
            symbol,
            kind,
            detail,
        } => Rendered {
            kind: "corporate_action",
            category: "execution",
            severity: Severity::Info,
            title_zh: format!(
                "{symbol} {}已入账",
                if kind == "split" { "拆股" } else { "分红" }
            ),
            title_en: format!(
                "{symbol} {} applied",
                if kind == "split" { "split" } else { "dividend" }
            ),
            body_zh: detail.clone(),
            body_en: detail.clone(),
            params: json!({"symbol": symbol, "kind": kind}),
            link: Some("/trades".into()),
        },
        Event::DailySummary {
            date,
            nav,
            day_pnl,
            day_return,
            total_return,
            trades,
            best,
            worst,
        } => {
            let movers_zh = match (best, worst) {
                (Some(b), Some(w)) => {
                    format!("\n领涨 {} {}，领跌 {} {}。", b.0, pct(b.1), w.0, pct(w.1))
                }
                _ => String::new(),
            };
            let movers_en = match (best, worst) {
                (Some(b), Some(w)) => {
                    format!("\nBest {} {}, worst {} {}.", b.0, pct(b.1), w.0, pct(w.1))
                }
                _ => String::new(),
            };
            Rendered {
                kind: "daily_summary",
                category: "report",
                severity: Severity::Info,
                title_zh: format!("{date} 收盘：{} ({})", money(*day_pnl), pct(*day_return)),
                title_en: format!("{date} close: {} ({})", money(*day_pnl), pct(*day_return)),
                body_zh: format!(
                    "组合净值 {}，累计收益 {}，今日成交 {trades} 笔。{movers_zh}",
                    money(*nav),
                    pct(*total_return)
                ),
                body_en: format!(
                    "NAV {}, total return {}, {trades} fills today.{movers_en}",
                    money(*nav),
                    pct(*total_return)
                ),
                params: json!({"date": date, "nav": nav, "day_return": day_return}),
                link: Some("/performance".into()),
            }
        }
        Event::WeeklyReport {
            week_end,
            week_return,
            total_return,
            max_drawdown,
            trades,
            nav,
        } => Rendered {
            kind: "weekly_report",
            category: "report",
            severity: Severity::Info,
            title_zh: format!("周报（截至 {week_end}）：本周 {}", pct(*week_return)),
            title_en: format!(
                "Weekly report (to {week_end}): {} this week",
                pct(*week_return)
            ),
            body_zh: format!(
                "净值 {}，累计 {}，最大回撤 {}，本周成交 {trades} 笔。",
                money(*nav),
                pct(*total_return),
                pct(*max_drawdown)
            ),
            body_en: format!(
                "NAV {}, total {}, max drawdown {}, {trades} fills this week.",
                money(*nav),
                pct(*total_return),
                pct(*max_drawdown)
            ),
            params: json!({"week_end": week_end}),
            link: Some("/performance".into()),
        },
        Event::PreOpen {
            trade_date,
            open_at,
            slots,
            mode,
        } => {
            let zh_slots: Vec<String> = slots
                .iter()
                .map(|(s, at)| format!("{} {}", slot_name(s, Zh), dual_time(*at, display)))
                .collect();
            let en_slots: Vec<String> = slots
                .iter()
                .map(|(s, at)| format!("{} {}", slot_name(s, En), dual_time(*at, display)))
                .collect();
            Rendered {
                kind: "pre_open",
                category: "reminder",
                severity: Severity::Info,
                title_zh: format!(
                    "美股 {trade_date} 将于 {} 开盘",
                    dual_time(*open_at, display)
                ),
                title_en: format!(
                    "US market opens {} on {trade_date}",
                    dual_time(*open_at, display)
                ),
                body_zh: format!(
                    "今日计划：{}。当前模式：{}。如需取消今日交易，请在计划生成前操作。",
                    zh_slots.join("；"),
                    mode_name(mode, Zh)
                ),
                body_en: format!(
                    "Today's plans: {}. Mode: {}. To cancel today's trading, do it before the plans are generated.",
                    en_slots.join("; "),
                    mode_name(mode, En)
                ),
                params: json!({"trade_date": trade_date}),
                link: Some("/".into()),
            }
        }
        Event::Reminder { title, note } => Rendered {
            kind: "reminder",
            category: "reminder",
            severity: Severity::Info,
            title_zh: format!("提醒：{title}"),
            title_en: format!("Reminder: {title}"),
            body_zh: note.clone(),
            body_en: note.clone(),
            params: json!({}),
            link: None,
        },
        Event::PlanReview {
            plan_id,
            slot,
            due,
            automatic,
        } => Rendered {
            kind: "plan_review",
            category: "reminder",
            severity: Severity::Info,
            title_zh: format!("请复核{}（#{plan_id}）", slot_name(slot, Zh)),
            title_en: format!("Review the {} (#{plan_id})", slot_name(slot, En)),
            body_zh: if *automatic {
                format!(
                    "计划将于 {} 自动执行；如需取消或删除个别订单，请在此之前操作。",
                    dual_time(*due, display)
                )
            } else {
                format!(
                    "计划等待你的确认，{} 未确认将自动过期。",
                    dual_time(*due, display)
                )
            },
            body_en: if *automatic {
                format!(
                    "It executes automatically at {}; cancel it or remove orders before then.",
                    dual_time(*due, display)
                )
            } else {
                format!(
                    "It needs your approval and expires at {}.",
                    dual_time(*due, display)
                )
            },
            params: json!({"plan_id": plan_id}),
            link: Some(format!("/plans/{plan_id}")),
        },
        Event::UniverseChanged { added, removed } => Rendered {
            kind: "universe_changed",
            category: "system",
            severity: Severity::Warning,
            title_zh: "投资范围已按 honeclaw 本体更新".into(),
            title_en: "Universe updated from the honeclaw ontology".into(),
            body_zh: format!(
                "新增：{}；移出：{}。移出的公司会在下一个计划中清仓。",
                list_or_none(added, Zh),
                list_or_none(removed, Zh)
            ),
            body_en: format!(
                "Added: {}; removed: {}. Removed companies are sold in the next plan.",
                list_or_none(added, En),
                list_or_none(removed, En)
            ),
            params: json!({"added": added, "removed": removed}),
            link: Some("/universe".into()),
        },
        Event::AccountReset {
            initial_cash,
            actor,
        } => Rendered {
            kind: "account_reset",
            category: "system",
            severity: Severity::Warning,
            title_zh: "模拟账户已重置".into(),
            title_en: "Paper account reset".into(),
            body_zh: format!(
                "{actor} 以 {} 初始资金新建了模拟账户，旧账户已归档。",
                money(*initial_cash)
            ),
            body_en: format!(
                "{actor} started a new paper account with {}; the old one is archived.",
                money(*initial_cash)
            ),
            params: json!({"initial_cash": initial_cash}),
            link: Some("/".into()),
        },
        Event::Test => Rendered {
            kind: "test",
            category: "system",
            severity: Severity::Info,
            title_zh: "测试通知".into(),
            title_en: "Test notification".into(),
            body_zh: "hone-quant 通知通道工作正常。".into(),
            body_en: "Your hone-quant notification channel works.".into(),
            params: json!({}),
            link: Some("/settings/notifications".into()),
        },
    }
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn reason_text(reason: &str, lang: Lang) -> String {
    if reason.trim().is_empty() {
        return String::new();
    }
    match lang {
        Lang::Zh => format!("原因：{reason}"),
        Lang::En => format!(" Reason: {reason}"),
    }
}

fn list_or_none(items: &[String], lang: Lang) -> String {
    if items.is_empty() {
        match lang {
            Lang::Zh => "无".into(),
            Lang::En => "none".into(),
        }
    } else {
        items.join(", ")
    }
}

fn outbound(row: &NotificationRow, lang: Lang, public_url: Option<&str>) -> OutboundMessage {
    let (title, body) = match lang {
        Lang::Zh => (row.title_zh.clone(), row.body_zh.clone()),
        Lang::En => (row.title_en.clone(), row.body_en.clone()),
    };
    OutboundMessage {
        title,
        body,
        severity: parse_severity(&row.severity),
        category: row.category.clone(),
        link: match (public_url, &row.link) {
            (Some(base), Some(path)) => Some(format!("{base}{path}")),
            _ => None,
        },
        timestamp: row.ts,
    }
}

/// Decrypted, enabled channels.
pub async fn load_channels(state: &AppState) -> Result<Vec<(String, ChannelConfig)>> {
    let client = state.pool.get().await?;
    let stored: ChannelMap = settings::get(&client, settings::CHANNELS).await?;
    let mut out = Vec::new();
    for (name, channel) in stored {
        if !channel.enabled {
            continue;
        }
        match state
            .secrets
            .open(&channel.sealed)
            .and_then(|bytes| Ok(serde_json::from_slice::<ChannelConfig>(&bytes)?))
        {
            Ok(config) => out.push((name, config)),
            Err(error) => tracing::warn!(channel = %name, %error, "cannot open stored channel"),
        }
    }
    Ok(out)
}

async fn deliver_all(state: &AppState, message: &OutboundMessage) -> Vec<Value> {
    let channels = match load_channels(state).await {
        Ok(channels) => channels,
        Err(error) => {
            tracing::warn!(%error, "cannot load notification channels");
            return Vec::new();
        }
    };
    let mut results = Vec::new();
    for (name, config) in channels {
        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            channels::send(&state.http, &config, message),
        )
        .await;
        let (ok, error) = match outcome {
            Ok(Ok(())) => (true, None),
            Ok(Err(e)) => (false, Some(e.to_string())),
            Err(_) => (false, Some("timed out".to_string())),
        };
        if let Some(error) = &error {
            tracing::warn!(channel = %name, %error, "notification delivery failed");
        }
        results.push(json!({"channel": name, "ok": ok, "error": error, "at": Utc::now()}));
    }
    results
}

/// Records an event, pushes it to browsers and (in the background) to outbound channels.
pub async fn notify(state: &std::sync::Arc<AppState>, event: Event) -> Result<i64> {
    let client = state.pool.get().await?;
    let display: DisplaySettings = settings::get(&client, settings::DISPLAY).await?;
    let prefs: NotificationSettings = settings::get(&client, settings::NOTIFICATIONS).await?;
    let rendered = render(&event, &display);
    let row = system::insert_notification(
        &client,
        rendered.kind,
        rendered.category,
        rendered.severity.as_str(),
        (&rendered.title_zh, &rendered.title_en),
        (&rendered.body_zh, &rendered.body_en),
        &rendered.params,
        rendered.link.as_deref(),
    )
    .await?;
    drop(client);
    state.emit(ServerEvent::Notification {
        id: row.id,
        severity: row.severity.clone(),
        category: row.category.clone(),
        title_zh: row.title_zh.clone(),
        title_en: row.title_en.clone(),
    });

    let wanted = prefs.category_enabled(&row.category)
        && severity_rank(rendered.severity) >= severity_rank(prefs.min_severity)
        && !matches!(event, Event::Test);
    if !wanted {
        return Ok(row.id);
    }
    let now = state.now();
    let quiet = prefs.quiet_hours.contains(now)
        && !(rendered.severity == Severity::Critical && prefs.critical_bypasses_quiet_hours);
    let id = row.id;
    if quiet {
        let client = state.pool.get().await?;
        system::set_deliveries(&client, id, &json!([]), true).await?;
        return Ok(id);
    }
    let state = state.clone();
    tokio::spawn(async move {
        let message = outbound(&row, prefs.language, state.config.public_url.as_deref());
        let results = deliver_all(&state, &message).await;
        if let Ok(client) = state.pool.get().await {
            let _ = system::set_deliveries(&client, id, &Value::Array(results), false).await;
        }
    });
    Ok(id)
}

/// Sends one test message to a single channel, bypassing routing and quiet hours.
pub async fn send_test(state: &AppState, config: &ChannelConfig, lang: Lang) -> Result<(), String> {
    let display = DisplaySettings::default();
    let rendered = render(&Event::Test, &display);
    let (title, body) = match lang {
        Lang::Zh => (rendered.title_zh, rendered.body_zh),
        Lang::En => (rendered.title_en, rendered.body_en),
    };
    let message = OutboundMessage {
        title,
        body,
        severity: Severity::Info,
        category: "system".into(),
        link: state.config.public_url.clone(),
        timestamp: state.now(),
    };
    channels::send(&state.http, config, &message)
        .await
        .map_err(|e| e.to_string())
}

/// Sends events held back by quiet hours as one digest per channel, once the window is over.
pub async fn flush_deferred(state: &std::sync::Arc<AppState>) -> Result<usize> {
    let client = state.pool.get().await?;
    let prefs: NotificationSettings = settings::get(&client, settings::NOTIFICATIONS).await?;
    if prefs.quiet_hours.contains(state.now()) {
        return Ok(0);
    }
    let display: DisplaySettings = settings::get(&client, settings::DISPLAY).await?;
    let pending = system::deferred_notifications(&client).await?;
    if pending.is_empty() {
        return Ok(0);
    }
    let lines: Vec<String> = pending
        .iter()
        .map(|n| {
            let title = match prefs.language {
                Lang::Zh => &n.title_zh,
                Lang::En => &n.title_en,
            };
            let at = n.ts.with_timezone(&zone(&display)).format("%m-%d %H:%M");
            format!("• {at} {title}")
        })
        .collect();
    let worst = pending
        .iter()
        .map(|n| parse_severity(&n.severity))
        .max_by_key(|s| severity_rank(*s))
        .unwrap_or(Severity::Info);
    let message = OutboundMessage {
        title: match prefs.language {
            Lang::Zh => format!("免打扰期间的 {} 条通知", pending.len()),
            Lang::En => format!("{} notifications from quiet hours", pending.len()),
        },
        body: lines.join("\n"),
        severity: worst,
        category: "report".into(),
        link: state
            .config
            .public_url
            .as_ref()
            .map(|base| format!("{base}/notifications")),
        timestamp: state.now(),
    };
    let results = deliver_all(state, &message).await;
    for n in &pending {
        let mut deliveries = results.clone();
        for d in deliveries.iter_mut() {
            d["digest"] = json!(true);
        }
        system::set_deliveries(&client, n.id, &Value::Array(deliveries), false).await?;
    }
    Ok(pending.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_and_percent_formatting() {
        assert_eq!(money(1_234_567.891), "$1,234,567.89");
        assert_eq!(money(-12.5), "-$12.50");
        assert_eq!(money(0.0), "$0.00");
        assert_eq!(money(999.999), "$1,000.00");
        assert_eq!(pct(0.01234), "+1.23%");
        assert_eq!(pct(-0.05), "-5.00%");
        assert_eq!(pct(0.0), "0.00%");
    }

    #[test]
    fn dual_time_shows_singapore_and_new_york() {
        let display = DisplaySettings::default();
        let at = DateTime::parse_from_rfc3339("2026-10-05T14:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(dual_time(at, &display), "22:00 SGT (10:00 ET)");
        let late = DateTime::parse_from_rfc3339("2026-10-05T17:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            dual_time(late, &display),
            "10-06 01:00 SGT (10-05 13:00 ET)"
        );
    }

    #[test]
    fn every_event_renders_in_both_languages() {
        let display = DisplaySettings::default();
        let now = Utc::now();
        let date = now.date_naive();
        let events = vec![
            Event::PlanGenerated {
                plan_id: 1,
                trade_date: date,
                slot: "open".into(),
                orders: 3,
                buys: 2,
                sells: 1,
                turnover: 0.05,
                execute_after: Some(now),
                deadline: now,
                strategy: "S".into(),
                preset_id: "sector_risk_budget".into(),
            },
            Event::PlanNoAction {
                plan_id: 1,
                trade_date: date,
                slot: "close".into(),
            },
            Event::PlanExecuted {
                plan_id: 1,
                trade_date: date,
                slot: "open".into(),
                filled: 3,
                rejected: 1,
                bought: 1.0,
                sold: 2.0,
                costs: 0.1,
            },
            Event::PlanFailed {
                plan_id: 1,
                slot: "open".into(),
                error: "x".into(),
            },
            Event::PlanCancelled {
                plan_id: 1,
                slot: "open".into(),
                actor: "a".into(),
                reason: "r".into(),
            },
            Event::PlanExpired {
                plan_id: 1,
                slot: "manual".into(),
            },
            Event::SlotSkipped {
                trade_date: date,
                slot: "open".into(),
                reason: "missed".into(),
            },
            Event::SlotsCancelled {
                trade_date: date,
                slots: vec!["open".into(), "close".into()],
                actor: "a".into(),
                reason: String::new(),
            },
            Event::StrategyActivated {
                version_id: 2,
                name: "n".into(),
                preset_id: "custom".into(),
                actor: "a".into(),
            },
            Event::AutomationChanged {
                mode: "paused".into(),
                paused_until: Some(now),
                actor: "a".into(),
            },
            Event::DrawdownAlert {
                drawdown: -0.12,
                threshold: 0.1,
            },
            Event::DailyLossAlert {
                loss: -0.04,
                threshold: 0.03,
            },
            Event::DataStale { minutes: 12 },
            Event::DataSyncFailed { error: "e".into() },
            Event::CorporateAction {
                symbol: "NVDA".into(),
                kind: "split".into(),
                detail: "d".into(),
            },
            Event::DailySummary {
                date,
                nav: 1.0,
                day_pnl: 1.0,
                day_return: 0.01,
                total_return: 0.1,
                trades: 4,
                best: Some(("A".into(), 0.05)),
                worst: Some(("B".into(), -0.03)),
            },
            Event::WeeklyReport {
                week_end: date,
                week_return: 0.01,
                total_return: 0.2,
                max_drawdown: -0.05,
                trades: 9,
                nav: 1.0,
            },
            Event::PreOpen {
                trade_date: date,
                open_at: now,
                slots: vec![("open".into(), now)],
                mode: "auto".into(),
            },
            Event::Reminder {
                title: "t".into(),
                note: "n".into(),
            },
            Event::PlanReview {
                plan_id: 3,
                slot: "close".into(),
                due: now,
                automatic: true,
            },
            Event::UniverseChanged {
                added: vec![],
                removed: vec!["X".into()],
            },
            Event::AccountReset {
                initial_cash: 1e6,
                actor: "a".into(),
            },
            Event::Test,
        ];
        for event in events {
            let r = render(&event, &display);
            assert!(
                !r.title_zh.is_empty() && !r.title_en.is_empty(),
                "{}",
                r.kind
            );
            assert!(
                !r.title_zh.is_ascii() || r.kind == "reminder",
                "{} zh title",
                r.kind
            );
            assert!(
                r.title_en.is_ascii() || r.kind == "reminder" || r.kind == "strategy_activated",
                "{} en title: {}",
                r.kind,
                r.title_en
            );
        }
    }

    #[test]
    fn severity_ordering() {
        assert!(severity_rank(Severity::Critical) > severity_rank(Severity::Warning));
        assert_eq!(
            severity_rank(Severity::Success),
            severity_rank(Severity::Info)
        );
    }
}
