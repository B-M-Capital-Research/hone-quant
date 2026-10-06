import { defineMessages } from "@/i18n";

const zh = {
  title: "登录 hone-quant",
  subtitle: "AI 基础设施组合的量化调仓台。所有交易均在模拟盘中进行。",
  username: "用户名",
  password: "密码",
  submit: "登录",
  submitting: "正在登录…",
  invalid: "用户名或密码错误。",
  rate_limited: "失败次数过多，已临时锁定。请 15 分钟后再试。",
  no_users_title: "尚未创建管理员",
  no_users_body: "在服务器上设置 HONE_QUANT_ADMIN_USER / HONE_QUANT_ADMIN_PASSWORD 后重启，或运行：",
  paper_note: "模拟盘 · 不连接任何实盘券商账户",
  footer: "数据与本体来自 honeclaw · 仅供研究与模拟",
  hc_title: "使用 hone-claw.com 管理员账号",
  hc_body: "hone-quant 与 hone-claw.com 共用账号，仅限管理员访问。请先在 hone-claw.com 登录，然后返回此页。",
  hc_sign_in: "前往 hone-claw.com 登录",
  hc_retry: "我已登录，继续",
  hc_checking: "正在确认登录状态…",
  hc_not_admin: "当前 hone-claw.com 账号不是管理员，无法访问 hone-quant。请换用管理员账号登录。",
  hc_unavailable: "暂时无法向 hone-claw.com 确认登录状态，请稍后重试。",
  hc_signed_out: "尚未检测到 hone-claw.com 的登录状态。",
};

const en: typeof zh = {
  title: "Sign in to hone-quant",
  subtitle: "Quantitative rebalancing for the AI-infrastructure portfolio. Every trade is simulated on a paper account.",
  username: "Username",
  password: "Password",
  submit: "Sign in",
  submitting: "Signing in…",
  invalid: "Incorrect username or password.",
  rate_limited: "Too many failed attempts — temporarily locked. Try again in 15 minutes.",
  no_users_title: "No administrator yet",
  no_users_body: "Set HONE_QUANT_ADMIN_USER / HONE_QUANT_ADMIN_PASSWORD on the server and restart, or run:",
  paper_note: "Paper trading · never connected to a live brokerage account",
  footer: "Universe and ontology from honeclaw · research and simulation only",
  hc_title: "Use your hone-claw.com administrator account",
  hc_body: "hone-quant shares accounts with hone-claw.com and is open to administrators only. Sign in at hone-claw.com, then come back to this page.",
  hc_sign_in: "Sign in at hone-claw.com",
  hc_retry: "I've signed in — continue",
  hc_checking: "Checking your sign-in…",
  hc_not_admin: "This hone-claw.com account is not an administrator and cannot use hone-quant. Sign in with an administrator account.",
  hc_unavailable: "Your hone-claw.com sign-in cannot be confirmed right now. Please try again shortly.",
  hc_signed_out: "No hone-claw.com sign-in was found.",
};

export const loginText = defineMessages(zh, en);
