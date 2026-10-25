/**
 * Shapes of the hone-quant API. Money and share quantities are exact decimals serialised as
 * strings (`Dec`); convert with `toNumber` from `lib/format`.
 */

export type Dec = string;
export type Iso = string;
export type DateStr = string;

export type DataSource = "fmp" | "demo";
export type MarketPhase = "pre_open" | "open" | "post_close" | "closed";
export type Slot = "open" | "close" | "manual";
export type PlanStatus =
  | "pending"
  | "executing"
  | "executed"
  | "partially_executed"
  | "no_action"
  | "cancelled"
  | "expired"
  | "skipped"
  | "failed";
export type OrderStatus = "planned" | "skipped" | "filled" | "partially_filled" | "rejected" | "cancelled" | "expired";
export type Side = "buy" | "sell";
export type OrderReason = "entry" | "exit" | "increase" | "decrease";
export type AutomationMode = "auto" | "approval" | "paused";
export type Severity = "info" | "success" | "warning" | "critical";
export type Role = "admin" | "member" | "viewer";

export interface DisplaySettings {
  timezone: string;
  up_color: "green_up" | "red_up";
}

export interface Meta {
  app: string;
  version: string;
  data_source: DataSource;
  demo: boolean;
  server_time: Iso;
  market_timezone: string;
  display: DisplaySettings;
  has_users: boolean;
  paper_only: boolean;
  /** How operators sign in: hone-quant accounts, or hone-claw.com administrators. */
  auth?: AuthInfo;
  base_path?: string;
}

export interface AuthInfo {
  mode: "local" | "honeclaw";
  /** Where to sign in to hone-claw.com (honeclaw mode). */
  login_url?: string;
}

export interface User {
  id?: number;
  username: string;
  /** Name to show; the audit identity is `username`. */
  display_name?: string;
  role: Role;
  /** Signed in through hone-claw.com rather than a hone-quant account. */
  external?: boolean;
  created_at?: Iso;
  last_login_at?: Iso | null;
}

export interface Session {
  date: DateStr;
  open: Iso;
  close: Iso;
  early_close: boolean;
}

export interface AutomationSettings {
  mode: AutomationMode;
  paused_until: Iso | null;
  note: string;
}

export interface Plan {
  id: number;
  account_id: number;
  trade_date: DateStr;
  slot: Slot;
  status: PlanStatus;
  strategy_version_id: number | null;
  automation_mode: AutomationMode;
  generated_at: Iso;
  execute_after: Iso | null;
  deadline: Iso;
  approved_at: Iso | null;
  approved_by: string | null;
  cancelled_at: Iso | null;
  cancelled_by: string | null;
  cancel_reason: string | null;
  executed_at: Iso | null;
  nav: Dec;
  cash: Dec;
  exposure_target: number | null;
  invested_target: number | null;
  breadth: number | null;
  est_vol: number | null;
  turnover: number;
  est_costs: Dec;
  order_count: number;
  summary: PlanSummary;
  error: string | null;
  created_by: string;
}

export interface PlanSummary {
  buys?: number;
  sells?: number;
  buy_value?: number;
  sell_value?: number;
  frozen?: number;
  excluded?: number;
  strategy_name?: string;
  cash_after?: number;
  skip_reason?: string;
  executed?: { filled: number; partial: number; rejected: number; bought: number; sold: number; costs: number; rejections?: Rejection[] };
}

/** What a rejected order saw against its execution limits (recorded from this version on). */
export interface Rejection {
  symbol: string;
  reason: string;
  ref_price: number;
  quote: number | null;
  quote_age_secs?: number | null;
  max_price_deviation?: number;
  max_quote_age_secs?: number;
  cash?: number;
}

export interface Order {
  id: number;
  plan_id: number;
  symbol: string;
  side: Side;
  reason: OrderReason;
  qty: Dec;
  ref_price: number;
  weight_before: number;
  weight_target: number;
  weight_after: number;
  status: OrderStatus;
  status_reason: string | null;
  filled_qty: Dec;
  sequence: number;
  created_at: Iso;
  updated_at: Iso;
}

export interface OrderWithPlan extends Order {
  trade_date: DateStr;
  slot: Slot;
}

export interface Fill {
  id: number;
  order_id: number;
  plan_id: number;
  symbol: string;
  side: Side;
  qty: Dec;
  price: Dec;
  quote_price: number;
  quote_ts: Iso | null;
  notional: Dec;
  commission: Dec;
  fees: Dec;
  slippage: Dec;
  realized_pnl: Dec | null;
  executed_at: Iso;
}

export interface Account {
  id: number;
  portfolio_id: number;
  name: string;
  base_currency: string;
  mode: "paper";
  initial_cash: Dec;
  cash: Dec;
  inception_date: DateStr;
  status: "active" | "archived";
  created_at: Iso;
  archived_at: Iso | null;
}

export interface PortfolioSummary {
  nav: number;
  cash: number;
  invested: number;
  total_return: number;
  day_return: number | null;
  /** Open positions. */
  positions: number;
  /** Of the active account. */
  initial_cash: number;
  /** Of the active account. */
  inception_date: DateStr;
  strategy: { id: number; name: string; preset_id: string } | null;
}

/** A named, isolated paper book: its own holdings, plans, fills, strategy version and automation. */
export interface Portfolio {
  id: number;
  name: string;
  description: string;
  /** Audit identity of the owner (local username or `honeclaw:<id>`); null = shared, managed by administrators. */
  owner: string | null;
  /** Display name of the owner; "" when shared. */
  owner_name: string;
  automation: AutomationSettings;
  effective_mode: AutomationMode;
  status: "active" | "archived";
  created_by: string;
  created_at: Iso;
  archived_at: Iso | null;
  /** The signed-in user may act on this portfolio (trade, rename, archive, reset). */
  can_trade: boolean;
  /** The active paper account; null when archived. */
  account: Account | null;
  /** Null for archived portfolios. */
  summary: PortfolioSummary | null;
}

export interface NewPortfolio {
  name: string;
  description?: string;
  initial_cash: number;
  strategy_version_id?: number | null;
  automation_mode?: AutomationMode;
  /** Admins only: a local username, or null for a shared portfolio. */
  owner?: string | null;
}

export interface PositionView {
  symbol: string;
  name_zh: string;
  name_en: string;
  sector_id: string;
  qty: number;
  avg_cost: number;
  price: number | null;
  price_source: "quote" | "close" | "none";
  prev_close: number | null;
  value: number;
  weight: number;
  cost_basis: number;
  unrealized_pnl: number;
  unrealized_pct: number | null;
  day_change_pct: number | null;
  day_pnl: number | null;
  realized_pnl: number;
  dividends: number;
  in_universe: boolean;
}

export interface Valuation {
  account: Account;
  as_of: Iso;
  cash: number;
  invested: number;
  nav: number;
  exposure: number;
  reference_date: DateStr | null;
  reference_nav: number | null;
  day_pnl: number | null;
  day_return: number | null;
  total_return: number;
  peak_nav: number;
  drawdown: number;
  realized_pnl: number;
  unrealized_pnl: number;
  dividends: number;
  positions: PositionView[];
}

export interface SkippedSlot {
  trade_date: DateStr;
  slot: "open" | "close";
  reason: string;
  created_by: string;
  created_at: Iso;
}

export interface SlotView {
  slot: "open" | "close";
  window_start: Iso;
  window_end: Iso;
  generate_at: Iso;
  execute_at: Iso;
  execute_deadline: Iso;
  plan: Plan | null;
  cancelled: SkippedSlot | null;
}

export interface MarketView {
  now: Iso;
  phase: MarketPhase;
  today: Session | null;
  next_session: Session;
  holiday: { id: string; name_zh: string; name_en: string } | null;
  schedule_date: DateStr;
  schedule: SlotView[];
  early_close: boolean;
  automation: AutomationSettings;
  effective_mode: AutomationMode;
}

export interface Restriction {
  id: number;
  /** null = applies to every portfolio. */
  portfolio_id: number | null;
  symbol: string;
  mode: "exclude" | "lock";
  reason: string;
  starts_on: DateStr;
  ends_on: DateStr | null;
  created_by: string;
  created_at: Iso;
  revoked_at: Iso | null;
  revoked_by: string | null;
}

export interface PlanWithOrders extends Plan {
  orders: Order[];
  headline: Headline | null;
}

export interface Dashboard {
  market: MarketView;
  valuation: Valuation;
  strategy: { id: number; name: string; preset_id: string; params: StrategyParams; created_at: Iso } | null;
  targets: { plan_id: number; generated_at: Iso; weights: Record<string, number> } | null;
  plans: PlanWithOrders[];
  recent_plans: Plan[];
  /** Breadth, exposure and next triggers from the latest plan; null before the first plan. */
  exposure: ExposureView | null;
  restrictions: Restriction[];
  unread_notifications: number;
  data: { source: DataSource; last_quote_at: Iso | null };
}

export interface ExposureView {
  plan_id: number;
  trade_date: DateStr;
  slot: Slot;
  generated_at: Iso;
  previous_plan_id: number | null;
  breadth_scaling: boolean;
  line: number;
  min_exposure: number;
  max_exposure: number;
  min_history_days: number;
  breadth: number | null;
  above: number | null;
  counted: number | null;
  exposure_target: number | null;
  invested_target: number | null;
  crossed_below: string[];
  crossed_above: string[];
  invested_now: number;
  cash_gap: number | null;
  cash_band: number;
  triggers: {
    invested: number;
    trim_below: number | null;
    top_up_above: number | null;
    counted: number | null;
    trim_below_count: number | null;
    top_up_above_count: number | null;
  } | null;
  history: { plan_id: number; trade_date: DateStr; slot: Slot; breadth: number | null; exposure_target: number | null; invested_target: number | null }[];
  bands: { symbol: string; weight: number; target: number; drift: number; band: number }[];
  weights: Record<string, { sector: string; sector_weight: number; share: number; vol: number | null; momentum_multiplier: number; trend_multiplier: number; breadth_gap: number | null; capped: boolean; target: number }>;
}

export interface Sector {
  id: string;
  name_zh: string;
  name_en: string;
  summary_zh: string;
  summary_en: string;
  sort_order: number;
}

export interface Asset {
  symbol: string;
  name_zh: string;
  name_en: string;
  sector_id: string;
  subtype_id: string;
  subtype_zh: string;
  subtype_en: string;
  also_in: string[];
  role_zh: string;
  sort_order: number;
  is_active: boolean;
}

export interface BoardItem {
  symbol: string;
  name_zh: string;
  name_en: string;
  sector_id: string;
  reference: number | null;
  open: number;
  high: number;
  low: number;
  close: number;
  volume: number;
  open_pct: number | null;
  high_pct: number | null;
  low_pct: number | null;
  close_pct: number | null;
  weight: number;
  target_weight: number | null;
  live: boolean;
}

export interface Board {
  period: string;
  from: DateStr;
  to: DateStr;
  live: boolean;
  as_of: Iso;
  sectors: Sector[];
  items: BoardItem[];
}

export interface Quote {
  symbol: string;
  price: number;
  change: number | null;
  change_pct: number | null;
  open: number | null;
  day_high: number | null;
  day_low: number | null;
  prev_close: number | null;
  volume: number | null;
  avg_volume: number | null;
  market_cap: number | null;
  timestamp: Iso | null;
  fetched_at?: Iso;
}

export interface Bar {
  t: string;
  o: number;
  h: number;
  l: number;
  c: number;
  v: number;
}

export interface Bars {
  symbol: string;
  range: string;
  interval: string;
  bars: Bar[];
  sma50: (number | null)[];
  sma200: (number | null)[];
  trades: { t: Iso; side: Side; qty: Dec; price: Dec; plan_id: number }[];
  quote: Quote | null;
  asset: Asset | null;
  benchmark: { symbol: string; name_zh: string; name_en: string } | null;
  position: { symbol: string; qty: Dec; avg_cost: Dec } | null;
  target_weight: number | null;
  live: boolean;
  display_timezone: string;
}

// ---------------------------------------------------------------------------------------------
// Strategy
// ---------------------------------------------------------------------------------------------

export interface StrategyParams {
  universe: { min_history_days: number };
  sector: {
    method: "risk" | "member_count" | "custom";
    vol_power: number;
    vol_lookback: number;
    momentum_tilt: number;
    momentum_lookback: number;
    momentum_skip: number;
    trend_aware: boolean;
    min_weight: number;
    max_weight: number;
    custom_budgets: Record<string, number>;
  };
  asset: {
    vol_power: number;
    vol_lookback: number;
    momentum_tilt: number;
    momentum_lookback: number;
    momentum_skip: number;
    trend_sma: number;
    trend_penalty: number;
    trend_ramp: number;
    min_weight: number;
    max_weight: number;
  };
  exposure: {
    max_exposure: number;
    min_exposure: number;
    breadth_scaling: boolean;
    breadth_sma: number;
    target_vol: number | null;
    vol_lookback: number;
  };
  rebalance: {
    band_abs: number;
    band_rel: number;
    min_trade_value: number;
    max_turnover: number;
    fractional_shares: boolean;
  };
  /** Absent on versions saved before the AI sector view existed (= off). */
  ai?: AiParams;
}

export type AiMode = "off" | "shadow" | "live";

export interface AiParams {
  mode: AiMode;
  max_tilt: number;
  min_confidence: number;
  max_age_days: number;
}

export interface Preset {
  id: string;
  name_zh: string;
  name_en: string;
  summary_zh: string;
  summary_en: string;
  params: StrategyParams;
}

export interface StrategyVersion {
  id: number;
  name: string;
  preset_id: string;
  params: StrategyParams;
  note: string;
  created_by: string;
  created_at: Iso;
}

export interface Activation {
  id: number;
  strategy_version_id: number;
  version_name: string;
  activated_by: string;
  note: string;
  activated_at: Iso;
}

export interface StrategyOverview {
  active: StrategyVersion | null;
  versions: StrategyVersion[];
  activations: Activation[];
  presets: Preset[];
  defaults: StrategyParams;
}

export type AssetStatus = "active" | "excluded" | "frozen" | "no_price" | "insufficient_history" | "below_min_weight";

export interface AssetDiagnostics {
  symbol: string;
  sector: string;
  status: AssetStatus;
  price: number | null;
  vol: number | null;
  momentum: number | null;
  momentum_rank: number | null;
  trend: number | null;
  /** price / SMA(breadth) − 1; absent on plans stored before it was recorded. */
  breadth_gap?: number | null;
  above_breadth_line?: boolean | null;
  momentum_multiplier: number;
  trend_multiplier: number;
  score: number;
  capped: boolean;
  weight: number;
}

export interface SectorDiagnostics {
  sector: string;
  active_members: number;
  vol: number | null;
  momentum: number | null;
  momentum_rank: number | null;
  trend_factor: number;
  /** Recorded from the AI sector view on. */
  ai_stance?: number | null;
  ai_multiplier?: number;
  score: number;
  cap: number;
  budget: number;
  weight: number;
}

export interface TargetResult {
  weights: number[];
  exposure_target: number;
  invested: number;
  cash_weight: number;
  breadth: number | null;
  est_vol: number | null;
  vol_scale: number | null;
  sectors: SectorDiagnostics[];
  assets: AssetDiagnostics[];
}

export interface Flag {
  symbol: string;
  reason: string;
}

export interface SkippedView {
  symbol: string;
  reason: "within_band" | "below_min_trade" | "not_tradable" | "rounded_to_zero";
  weight_before: number;
  weight_target: number;
}

export interface ProposedOrder {
  symbol: string;
  side: Side;
  reason: OrderReason;
  qty: number;
  price: number;
  notional: number;
  weight_before: number;
  weight_target: number;
  weight_after: number;
}

export interface Preview {
  /** Compared with the portfolio's latest plan, as a plan would be. */
  explanation?: PlanExplanation | null;
  as_of: Iso;
  nav: number;
  cash: number;
  targets: TargetResult;
  orders: ProposedOrder[];
  skipped: SkippedView[];
  frozen: Flag[];
  excluded: Flag[];
  turnover: number;
  est_costs: number;
  history: { from: DateStr | null; to: DateStr | null };
  /** What the AI sector view did, when the parameters consult it. */
  ai?: PlanAi | null;
}

export interface PlanDiagnostics {
  targets?: TargetResult;
  skipped?: SkippedView[];
  frozen?: Flag[];
  excluded?: Flag[];
  quotes?: Record<string, { price: number; quote_ts: Iso | null; age_secs: number }>;
  params?: StrategyParams;
  strategy?: { id: number; name: string; preset_id: string } | null;
  rebalance?: {
    nav: number;
    turnover: number;
    buy_value: number;
    sell_value: number;
    est_commission: number;
    est_slippage: number;
    est_fees: number;
    cash_after: number;
    turnover_scale: number | null;
    cash_scale: number | null;
    /** Recorded from this version on. */
    invested_now?: number;
    invested_target?: number;
    cash_gap?: number;
    cash_band?: number;
    band_waiver?: BandWaiver | null;
    band_waived?: string[];
  };
  history?: { from: DateStr | null; to: DateStr | null };
  explain?: PlanExplanation;
  /** What the AI sector view did; absent when the strategy does not consult it. */
  ai?: PlanAi | null;
}

// ---------------------------------------------------------------------------------------------
// AI sector view
// ---------------------------------------------------------------------------------------------

export interface PlanAiSector {
  sector: string;
  stance: number;
  confidence: number;
  multiplier: number;
  weight_with: number;
  weight_without: number;
}

export interface PlanAi {
  mode: AiMode;
  /** `live`: traded with the view · `shadow`: recorded only · `unavailable`: no usable view. */
  status: "live" | "shadow" | "unavailable";
  view_id: number | null;
  as_of: DateStr | null;
  model: string | null;
  formed_at: Iso | null;
  max_tilt: number;
  min_confidence: number;
  max_age_days: number;
  sectors: PlanAiSector[];
  weights_with: Record<string, number>;
  weights_without: Record<string, number>;
  active_share: number;
}

export interface AiEvidence {
  source: "ontology" | "earnings" | "market";
  ref: string;
  date: DateStr | null;
  note_zh: string;
  note_en: string;
}

export interface AiSectorOpinion {
  sector: string;
  stance: number;
  confidence: number;
  rationale_zh: string;
  rationale_en: string;
  risks_zh: string;
  risks_en: string;
  evidence: AiEvidence[];
  proposed_stance: number | null;
  flags: string[];
}

export interface AiViewOutput {
  as_of: DateStr;
  summary_zh: string;
  summary_en: string;
  sectors: AiSectorOpinion[];
  notes: string[];
}

export interface AiView {
  id: number;
  as_of: DateStr;
  status: "running" | "ok" | "failed";
  prompt_version: string;
  model: string;
  output: AiViewOutput | null;
  error: string | null;
  usage: Record<string, unknown> | null;
  latency_ms: number | null;
  created_by: string;
  created_at: Iso;
  material?: unknown;
  raw_response?: string | null;
}

export interface AiOverview {
  configured: boolean;
  model: string | null;
  endpoint: string | null;
  origin: string | null;
  ontology: string;
  running: boolean;
  attempts_24h: number;
  max_attempts_24h: number;
  portfolios: { portfolio_id: number; portfolio: string; version_id: number; version: string; ai: AiParams }[];
  latest: AiView | null;
  views: AiView[];
}

export interface AiOverlayDay {
  date: DateStr;
  bps: number;
  cumulative_bps: number;
  plan_id: number;
}

export interface AiEvaluation {
  plans: number;
  days: number;
  first_plan: DateStr | null;
  cumulative_bps: number;
  mean_bps: number | null;
  hit_rate: number | null;
  t_stat: number | null;
  mean_active_share: number | null;
  sectors: Record<string, number>;
  series: AiOverlayDay[];
}

// ---------------------------------------------------------------------------------------------
// Plan explanations (quant_core::explain)
// ---------------------------------------------------------------------------------------------

export type BandWaiver = "buys" | "sells";

export type OrderDriver =
  | "first_plan"
  | "newly_eligible"
  | "added_to_universe"
  | "back_above_min_weight"
  | "restriction_lifted"
  | "trend_reentry"
  | "below_min_weight"
  | "trend_exit"
  | "excluded"
  | "not_in_universe"
  | "lost_eligibility"
  | "exposure"
  | "sector_budget"
  | "trend"
  | "momentum"
  | "volatility"
  | "peers"
  | "new_peers"
  | "price_drift"
  | "carry_over_turnover"
  | "carry_over_cash"
  | "retry";

export type NoActionReason = "within_bands" | "below_min_trade" | "not_tradable" | "nothing_to_trade";

/** Orders attributed to a plan-level driver and the weight they trade. */
interface Tally {
  orders: number;
  weight: number;
}

export type PlanDriver =
  | { code: "first_plan"; eligible: number; universe: number }
  | { code: "strategy_changed"; from: number | null; to: number | null }
  | { code: "history_limited"; insufficient: number; universe: number }
  | { code: "no_action"; reason: NoActionReason; gap: number }
  | ({ code: "newly_eligible"; count: number; cut: number } & Tally)
  | ({ code: "breadth"; line: number; before: number; now: number; crossed_below: number; crossed_above: number; exposure_before: number; exposure_now: number } & Tally)
  | ({ code: "trend_breaks"; count: number } & Tally)
  | ({ code: "trend_recoveries"; count: number } & Tally)
  | ({ code: "sector_budget"; sector: string; before: number; now: number } & Tally)
  | ({ code: "orders"; driver: OrderDriver } & Tally)
  | ({ code: "price_drift" } & Tally)
  | ({ code: "carry_over"; previous_turnover_scale: number | null; previous_cash_scale: number | null } & Tally)
  | ({ code: "retry" } & Tally)
  | { code: "turnover_cap"; scale: number; limit: number }
  | { code: "cash_cap"; scale: number }
  | { code: "band_waiver"; side: BandWaiver; gap: number; invested_now: number; invested_target: number; orders: number }
  | { code: "frozen"; count: number }
  | { code: "excluded"; count: number };

export interface Headline {
  previous_plan_id: number | null;
  drivers: PlanDriver[];
}

export interface OrderExplanation {
  symbol: string;
  sector: string | null;
  side: Side;
  weight_before: number;
  weight_target: number;
  previous_target: number | null;
  drift: number | null;
  model: number | null;
  driver: OrderDriver;
  status_before: AssetStatus | null;
  status_now: AssetStatus | null;
  components: { driver: OrderDriver; log: number; weight: number }[];
  price_drift: number | null;
  left_over: number | null;
  left_over_cause: OrderDriver | null;
  previous_order_status: OrderStatus | null;
  previous_status_reason: string | null;
  price_change: number | null;
  trend_multiplier_before: number | null;
  trend_multiplier_now: number | null;
  band_waived: boolean;
  scaled: boolean;
}

export interface PlanExplanation extends Headline {
  version: number;
  breadth: {
    source: "recorded" | "reconstructed" | "unavailable";
    before: number | null;
    now: number | null;
    above_before: number | null;
    counted_before: number | null;
    above_now: number | null;
    counted_now: number | null;
    crossed_below: string[];
    crossed_above: string[];
  };
  exposure: { before: number | null; now: number | null };
  est_vol: { before: number | null; now: number | null };
  cash: { invested_now: number; invested_target: number; gap: number; band: number; waiver: BandWaiver | null; derived: boolean };
  turnover_scale: number | null;
  cash_scale: number | null;
  universe: number;
  insufficient_history: number;
  newly_eligible: string[];
  trend_breaks: { symbol: string; before: number; now: number }[];
  trend_recoveries: { symbol: string; before: number; now: number }[];
  sectors: { sector: string; before: number | null; now: number; share_effect: number | null }[];
  frozen: Flag[];
  excluded: Flag[];
  no_action: {
    reason: NoActionReason;
    within_band: number;
    below_min_trade: number;
    not_tradable: number;
    rounded: number;
    closest: { symbol: string; drift: number; band: number } | null;
  } | null;
  tally: { driver: OrderDriver; orders: number; weight: number }[];
  orders: OrderExplanation[];
}

export interface AuditEntry {
  id: number;
  ts: Iso;
  actor: string;
  action: string;
  entity_type: string;
  entity_id: string;
  detail: Record<string, unknown>;
  ip: string;
}

export interface PlanDetail {
  plan: Plan;
  orders: Order[];
  fills: Fill[];
  diagnostics: PlanDiagnostics;
  /** Why this plan: stored at generation, or computed on read for older plans. */
  explanation: PlanExplanation | null;
  audit: AuditEntry[];
  strategy: StrategyVersion | null;
  names: Record<string, { zh: string; en: string; sector: string }>;
  /** The portfolio the plan belongs to (which may differ from the current one). */
  portfolio: { id: number; name: string };
  /** The signed-in user may approve, cancel or edit this plan. */
  can_trade: boolean;
}

// ---------------------------------------------------------------------------------------------
// Research
// ---------------------------------------------------------------------------------------------

export interface PerformanceMetrics {
  start: DateStr | null;
  end: DateStr | null;
  trading_days: number;
  annualisation_reliable: boolean;
  total_return: number;
  cagr: number | null;
  ann_vol: number | null;
  sharpe: number | null;
  sortino: number | null;
  max_drawdown: number;
  max_drawdown_peak: DateStr | null;
  max_drawdown_trough: DateStr | null;
  max_drawdown_recovery: DateStr | null;
  longest_drawdown_days: number;
  calmar: number | null;
  best_day: number | null;
  worst_day: number | null;
  hit_rate: number | null;
  var_95: number | null;
  cvar_95: number | null;
  skew: number | null;
  benchmark_total_return: number | null;
  excess_return: number | null;
  beta: number | null;
  alpha: number | null;
  correlation: number | null;
  tracking_error: number | null;
  information_ratio: number | null;
}

export interface PeriodReturn {
  year: number;
  month: number | null;
  ret: number;
}

export interface Attribution {
  key: string;
  sector: string | null;
  contribution: number;
  avg_weight: number;
}

export interface Performance {
  dates: DateStr[];
  nav: number[];
  benchmarks: { symbol: string; values: (number | null)[]; metrics: PerformanceMetrics | null }[];
  primary_benchmark: string;
  metrics: PerformanceMetrics;
  drawdowns: number[];
  monthly_returns: PeriodReturn[];
  yearly_returns: PeriodReturn[];
  rolling_vol: (number | null)[];
  asset_attribution: Attribution[];
  sector_attribution: Attribution[];
  trades: {
    fills: number;
    buys: number;
    sells: number;
    bought: number;
    sold: number;
    commissions: number;
    fees: number;
    slippage: number;
    realized_pnl: number;
    winning_sells: number;
    losing_sells: number;
    annual_turnover: number | null;
  };
  inception_date: DateStr;
}

export interface CostModel {
  commission_per_share: number;
  commission_min: number;
  commission_max_rate: number;
  commission_rate: number;
  slippage_bps: number;
  sell_fee_rate: number;
}

export interface BacktestConfig {
  start: DateStr;
  end: DateStr;
  initial_cash: number;
  params: StrategyParams;
  costs: CostModel;
  slots: ("open" | "close")[];
  frequency: "daily" | "weekly" | "monthly";
  risk_free_rate: number;
  benchmark: string | null;
}

export interface BacktestRow {
  id: number;
  name: string;
  status: "queued" | "running" | "succeeded" | "failed";
  config: BacktestConfig;
  strategy_version_id: number | null;
  summary: {
    metrics: PerformanceMetrics;
    final_nav: number | null;
    total_costs: number;
    annual_turnover: number;
    trades: number;
    benchmarks: { symbol: string; total_return: number; cagr: number | null; max_drawdown: number; sharpe: number | null }[];
    elapsed_ms: number;
  } | null;
  error: string | null;
  created_by: string;
  created_at: Iso;
  started_at: Iso | null;
  finished_at: Iso | null;
}

export type BacktestWarning =
  | { code: "survivorship_bias" }
  | { code: "late_listings"; symbols: [string, DateStr][] }
  | { code: "missing_data"; symbols: string[] }
  | { code: "short_period" };

export interface BacktestResult {
  points: { date: DateStr; nav: number; cash: number; invested: number; turnover: number; costs: number; trades: number }[];
  metrics: PerformanceMetrics;
  benchmarks: { symbol: string; values: number[]; metrics: PerformanceMetrics }[];
  drawdowns: number[];
  monthly_returns: PeriodReturn[];
  yearly_returns: PeriodReturn[];
  sector_weights: { date: DateStr; weights: Record<string, number>; cash: number }[];
  asset_contributions: Attribution[];
  sector_contributions: Attribution[];
  trades: {
    date: DateStr;
    slot: "open" | "close";
    symbol: string;
    side: Side;
    reason: OrderReason;
    qty: number;
    price: number;
    notional: number;
    cost: number;
  }[];
  final_positions: { symbol: string; sector: string; qty: number; value: number; weight: number }[];
  total_costs: number;
  total_turnover: number;
  annual_turnover: number;
  rebalances: number;
  warnings: BacktestWarning[];
  total_trades: number;
  elapsed_ms: number;
}

// ---------------------------------------------------------------------------------------------
// System
// ---------------------------------------------------------------------------------------------

export interface NotificationRow {
  id: number;
  ts: Iso;
  kind: string;
  category: string;
  severity: Severity;
  title_zh: string;
  title_en: string;
  body_zh: string;
  body_en: string;
  params: Record<string, unknown>;
  link: string | null;
  /** null = not about a specific portfolio. */
  portfolio_id: number | null;
  portfolio_name: string | null;
  read_at: Iso | null;
  deliveries: { channel: string; ok: boolean; error: string | null; at: Iso; digest?: boolean }[];
  deferred: boolean;
}

export type ReminderSchedule =
  | { type: "pre_open"; minutes_before: number }
  | { type: "post_close"; minutes_after: number }
  | { type: "week_close"; minutes_after: number }
  | { type: "before_deadline"; minutes_before: number }
  | { type: "once"; at: Iso }
  | { type: "daily"; time: string; tz: string }
  | { type: "trading_days"; time: string; tz: string }
  | { type: "weekly"; time: string; tz: string; weekdays: number[] };

export interface Reminder {
  id: number;
  kind: string;
  title: string;
  note: string;
  schedule: ReminderSchedule;
  enabled: boolean;
  last_fired_at: Iso | null;
  next_fire_at: Iso | null;
  created_by: string;
  created_at: Iso;
  updated_at: Iso;
}

export interface ScheduleSettings {
  open_offset_minutes: number;
  close_offset_minutes: number;
  review_minutes: number;
  min_gap_minutes: number;
}

export interface ExecutionSettings {
  costs: CostModel;
  max_quote_age_secs: number;
  max_price_deviation: number;
  max_daily_move: number;
  quote_poll_secs: number;
}

export interface RiskSettings {
  drawdown_alert: number;
  daily_loss_alert: number;
}

export interface NotificationSettings {
  language: "zh" | "en";
  quiet_hours: { enabled: boolean; start: string; end: string; timezone: string };
  critical_bypasses_quiet_hours: boolean;
  categories: Record<string, boolean>;
  min_severity: Severity;
  browser: boolean;
}

export interface BenchmarkSettings {
  symbols: string[];
  primary: string;
  risk_free_rate: number;
}

export interface SettingsBundle {
  schedule: ScheduleSettings;
  execution: ExecutionSettings;
  risk: RiskSettings;
  notifications: NotificationSettings;
  display: DisplaySettings;
  benchmarks: BenchmarkSettings;
  defaults: {
    schedule: ScheduleSettings;
    execution: ExecutionSettings;
    risk: RiskSettings;
    notifications: NotificationSettings;
    display: DisplaySettings;
    benchmarks: BenchmarkSettings;
  };
}

export type ChannelKind = "telegram" | "feishu" | "wecom" | "slack" | "discord" | "webhook" | "email";

export type ChannelConfig =
  | { kind: "telegram"; bot_token: string; chat_id: string }
  | { kind: "feishu"; webhook_url: string; secret?: string | null }
  | { kind: "wecom"; webhook_url: string }
  | { kind: "slack"; webhook_url: string }
  | { kind: "discord"; webhook_url: string }
  | { kind: "webhook"; url: string; secret?: string | null }
  | {
      kind: "email";
      host: string;
      port: number;
      username?: string | null;
      password?: string | null;
      from: string;
      to: string[];
      tls: "start_tls" | "implicit" | "none";
    };

export interface ChannelRow {
  name: string;
  kind: ChannelKind;
  label: string;
  enabled: boolean;
  config: ChannelConfig | null;
  readable: boolean;
  updated_at: Iso;
}

export interface JobRun {
  id: number;
  job: string;
  run_key: string;
  status: "running" | "succeeded" | "failed" | "skipped";
  started_at: Iso;
  finished_at: Iso | null;
  detail: Record<string, unknown>;
  error: string | null;
}

export interface Coverage {
  symbol: string;
  first: DateStr | null;
  last: DateStr | null;
  bars: number;
  adjusted: number;
}

export interface DataStatus {
  source: DataSource;
  fmp: { keys: number; key_origin: string; base_url: string; api_mode: string; requests_per_minute: number };
  history_years: number;
  coverage: Coverage[];
  quotes: { count: number; newest: Iso | null; oldest: Iso | null };
  jobs: Record<string, JobRun | null>;
  database: string;
  database_borrowed_from_honeclaw: boolean;
}

export interface UniverseVersion {
  id: number;
  source: string;
  ontology_schema_version: number | null;
  ontology_generated_at: string | null;
  content_hash: string;
  changes: UniverseChanges;
  applied_by: string;
  applied_at: Iso;
}

export interface UniverseChanges {
  added: string[];
  removed: string[];
  moved: [string, string, string][];
  sectors_added: string[];
  sectors_removed: string[];
  first_load: boolean;
}

export interface UniverseView {
  sectors: Sector[];
  assets: Asset[];
  removed: Asset[];
  benchmarks: { symbol: string; name_zh: string; name_en: string }[];
  versions: UniverseVersion[];
  restrictions: Restriction[];
  bundled_source: {
    kind: string;
    location: string;
    ontology_schema_version: number | null;
    ontology_generated_at: string | null;
    edits_applied: number;
    built_at: string;
  };
  ontology_url: string;
}

export interface LedgerEntry {
  id: number;
  ts: Iso;
  kind: "deposit" | "trade" | "dividend" | "adjustment";
  amount: Dec;
  balance_after: Dec;
  symbol: string | null;
  ref_type: string | null;
  ref_id: string | null;
  note: string;
}

export interface FieldError {
  path: string;
  code: string;
  min?: number;
  max?: number;
  message: string;
}

export type ServerEvent =
  | { type: "quotes"; at: Iso }
  | { type: "plan"; plan_id: number; status: PlanStatus; portfolio_id: number }
  | { type: "account"; reason: string; portfolio_id: number }
  | { type: "notification"; id: number; severity: Severity; category: string; title_zh: string; title_en: string }
  | { type: "backtest"; id: number; status: string }
  /** automation / restrictions / trading_day carry their portfolio; global settings carry null. */
  | { type: "settings"; key: string; portfolio_id: number | null }
  | { type: "strategy"; version_id: number; portfolio_id: number | null }
  /** A portfolio was created, renamed or archived. */
  | { type: "portfolios"; portfolio_id: number }
  | { type: "universe" }
  /** An AI sector view was formed (or failed). */
  | { type: "ai_view"; id: number; status: "running" | "ok" | "failed" }
  | { type: "resync" }
  | { type: "hello" };
