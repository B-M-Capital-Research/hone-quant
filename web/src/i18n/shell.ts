import { defineMessages } from "@/i18n";

const zh = {
  next: {
    opens_in: "距开盘",
    closes_in: "距收盘",
    plan_in: "{slot}生成",
    executes_in: "{slot}执行",
    approval_due: "{slot}待确认",
    next_session: "下一交易日",
    holiday: "今日休市：{name}",
    early_close: "今日 13:00 ET 提前收盘",
  },
  automation: {
    label: "自动交易",
    change_title: "切换自动交易模式",
    pause_session: "暂停至 {date} 收盘（跳过该交易日剩余计划）",
    pause_until: "暂停至 {time}",
    resume: "恢复",
    note_placeholder: "可选：说明原因（记录到审计日志）",
    confirm_auto: "切换为自动执行：之后生成的计划会在复核窗口结束后自动成交。",
    confirm_approval: "切换为人工确认：之后的计划需要你手动确认，未确认将在截止时自动过期。",
    confirm_paused: "暂停自动交易：不再生成计划，已生成的待执行计划不受影响（可单独取消）。",
    scope: "只作用于组合「{name}」，其他组合不受影响。",
  },
  user: {
    signed_in_as: "当前登录",
    via_honeclaw: "通过 hone-claw.com 管理员账号登录",
    honeclaw_account: "hone-claw.com 账号",
  },
  connection: {
    live: "实时连接",
    reconnecting: "正在重连…",
  },
};

const en: typeof zh = {
  next: {
    opens_in: "Opens in",
    closes_in: "Closes in",
    plan_in: "{slot} in",
    executes_in: "{slot} executes in",
    approval_due: "{slot} awaits approval",
    next_session: "Next session",
    holiday: "Market closed: {name}",
    early_close: "Early close today at 13:00 ET",
  },
  automation: {
    label: "Automation",
    change_title: "Change automation mode",
    pause_session: "Pause through the {date} close (skips that session's remaining plans)",
    pause_until: "Pause until {time}",
    resume: "Resume",
    note_placeholder: "Optional: why (recorded in the audit log)",
    confirm_auto: "Switch to automatic: new plans execute on their own after the review window.",
    confirm_approval: "Switch to approval: new plans wait for your approval and expire at their deadline otherwise.",
    confirm_paused: "Pause automation: no new plans are generated. Pending plans are not affected (cancel them separately).",
    scope: "Applies to “{name}” only; other portfolios are not affected.",
  },
  user: {
    signed_in_as: "Signed in as",
    via_honeclaw: "Signed in with a hone-claw.com administrator account",
    honeclaw_account: "hone-claw.com account",
  },
  connection: {
    live: "Live",
    reconnecting: "Reconnecting…",
  },
};

export const shellText = defineMessages(zh, en);
