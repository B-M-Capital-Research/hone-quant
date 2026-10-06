/**
 * Typed client for the hone-quant API. Every mutating request carries `X-Hone-Quant-Action`
 * (the server's CSRF guard) and errors surface as `ApiError` with the server's machine code.
 */
import { withBase } from "@/lib/base";
import type * as T from "@/lib/types";

export class ApiError extends Error {
  constructor(
    public status: number,
    public code: string,
    message: string,
    public fields?: T.FieldError[],
  ) {
    super(message);
  }
}

let unauthorizedHandler: (() => void) | null = null;

export function onUnauthorized(handler: () => void) {
  unauthorizedHandler = handler;
}

async function request<R>(method: string, path: string, body?: unknown): Promise<R> {
  const headers: Record<string, string> = { Accept: "application/json" };
  if (method !== "GET") headers["X-Hone-Quant-Action"] = "1";
  if (body !== undefined) headers["Content-Type"] = "application/json";
  let response: Response;
  try {
    response = await fetch(withBase(`/api${path}`), {
      method,
      headers,
      credentials: "same-origin",
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch {
    throw new ApiError(0, "network", "network");
  }
  let data: any = null;
  const text = await response.text();
  if (text) {
    try {
      data = JSON.parse(text);
    } catch {
      data = null;
    }
  }
  if (!response.ok) {
    if (response.status === 401 && path !== "/auth/login") unauthorizedHandler?.();
    throw new ApiError(response.status, data?.error ?? "http", data?.message ?? response.statusText, data?.fields);
  }
  return data as R;
}

const get = <R>(path: string) => request<R>("GET", path);
const post = <R>(path: string, body?: unknown) => request<R>("POST", path, body ?? {});
const put = <R>(path: string, body: unknown) => request<R>("PUT", path, body);
const del = <R>(path: string) => request<R>("DELETE", path);

function qs(params: Record<string, string | number | boolean | null | undefined>): string {
  const entries = Object.entries(params).filter(([, v]) => v !== undefined && v !== null && v !== "");
  if (!entries.length) return "";
  return "?" + entries.map(([k, v]) => `${encodeURIComponent(k)}=${encodeURIComponent(String(v))}`).join("&");
}

export const api = {
  // Session
  meta: () => get<T.Meta>("/meta"),
  login: (username: string, password: string) => post<{ user: T.User }>("/auth/login", { username, password }),
  logout: () => post<{ ok: boolean }>("/auth/logout"),
  me: () => get<{ user: T.User }>("/auth/me"),
  changePassword: (current: string, next: string) => post<{ ok: boolean }>("/auth/password", { current, new: next }),
  users: () => get<{ users: T.User[] }>("/users"),
  createUser: (username: string, password: string, role: T.Role) => post<{ user: T.User }>("/users", { username, password, role }),
  deleteUser: (id: number) => del<{ ok: boolean }>(`/users/${id}`),

  // Overview & market
  dashboard: () => get<T.Dashboard>("/dashboard"),
  market: () => get<T.MarketView>("/market"),
  board: (period: string) => get<T.Board>(`/board${qs({ period })}`),
  bars: (symbol: string, range: string) => get<T.Bars>(`/bars/${encodeURIComponent(symbol)}${qs({ range })}`),
  quotes: () => get<{ quotes: Record<string, T.Quote> }>("/quotes"),

  // Plans & trading
  plans: (q: { from?: string; to?: string; status?: string; limit?: number; offset?: number }) =>
    get<{ plans: T.Plan[]; total: number }>(`/plans${qs(q)}`),
  plan: (id: number) => get<T.PlanDetail>(`/plans/${id}`),
  approvePlan: (id: number, note = "") => post<{ report: unknown }>(`/plans/${id}/approve`, { note }),
  cancelPlan: (id: number, reason = "") => post<{ ok: boolean }>(`/plans/${id}/cancel`, { reason }),
  skipOrder: (planId: number, orderId: number) => post<{ ok: boolean }>(`/plans/${planId}/orders/${orderId}/skip`),
  generatePlan: () => post<{ plan: { plan_id: number; status: string; orders: number } }>("/plans/generate"),
  tradingDay: (date: string) => get<any>(`/trading-days/${date}`),
  cancelDay: (date: string, slots: string[], reason: string) =>
    post<{ cancelled_plans: number[]; pre_cancelled: string[]; already_final: unknown[] }>(`/trading-days/${date}/cancel`, { slots, reason }),
  restoreSlot: (date: string, slot: string) => del<{ ok: boolean }>(`/trading-days/${date}/cancel/${slot}`),
  automation: () => get<{ automation: T.AutomationSettings; effective_mode: T.AutomationMode }>("/automation"),
  setAutomation: (value: T.AutomationSettings) => put<{ automation: T.AutomationSettings }>("/automation", value),
  restrictions: () => get<{ active: T.Restriction[]; history: T.Restriction[] }>("/restrictions"),
  addRestriction: (body: { symbol: string; mode: "exclude" | "lock"; reason: string; ends_on?: string | null }) =>
    post<{ restriction: T.Restriction }>("/restrictions", body),
  revokeRestriction: (id: number) => del<{ restriction: T.Restriction }>(`/restrictions/${id}`),
  orders: (q: { symbol?: string; from?: string; to?: string; status?: string; limit?: number; offset?: number }) =>
    get<{ orders: T.OrderWithPlan[]; total: number }>(`/orders${qs(q)}`),
  fills: (q: { symbol?: string; from?: string; to?: string; limit?: number; offset?: number }) =>
    get<{ fills: T.Fill[]; total: number }>(`/fills${qs(q)}`),
  ledger: (limit = 500) => get<{ entries: T.LedgerEntry[] }>(`/ledger${qs({ limit })}`),
  accounts: () => get<{ accounts: T.Account[] }>("/accounts"),
  resetAccount: (initial_cash: number) => post<unknown>("/account/reset", { initial_cash, confirm: "RESET" }),

  // Strategy & universe
  strategy: () => get<T.StrategyOverview>("/strategy"),
  previewStrategy: (params: T.StrategyParams) => post<T.Preview>("/strategy/preview", { params }),
  createVersion: (body: { name: string; preset_id: string; params: T.StrategyParams; note: string; activate: boolean }) =>
    post<{ version: T.StrategyVersion; activated: boolean }>("/strategy/versions", body),
  activateVersion: (id: number, note = "") => post<{ version: T.StrategyVersion }>(`/strategy/versions/${id}/activate`, { note }),
  universe: () => get<T.UniverseView>("/universe"),
  universeCheck: (ontology?: string, edits?: string) =>
    post<{ changes: T.UniverseChanges; source: unknown; sectors: number; assets: number }>("/universe/check", { ontology, edits }),
  universeApply: (ontology?: string, edits?: string) => post<{ changes: T.UniverseChanges }>("/universe/apply", { ontology, edits }),

  // Research
  backtests: () => get<{ backtests: T.BacktestRow[] }>("/backtests"),
  backtest: (id: number) => get<{ backtest: T.BacktestRow; result: T.BacktestResult | null }>(`/backtests/${id}`),
  createBacktest: (body: Record<string, unknown>) => post<{ backtest: T.BacktestRow }>("/backtests", body),
  deleteBacktest: (id: number) => del<{ ok: boolean }>(`/backtests/${id}`),
  performance: (range: string) => get<T.Performance>(`/performance${qs({ range })}`),

  // System
  notifications: (q: { unread?: boolean; category?: string; before?: number; limit?: number }) =>
    get<{ notifications: T.NotificationRow[]; unread: number }>(`/notifications${qs(q)}`),
  readNotification: (id: number) => post<{ unread: number }>(`/notifications/${id}/read`),
  readAllNotifications: () => post<{ unread: number }>("/notifications/read-all"),
  reminders: () => get<{ reminders: T.Reminder[] }>("/reminders"),
  createReminder: (body: { title: string; note: string; schedule: T.ReminderSchedule; enabled: boolean }) =>
    post<{ reminder: T.Reminder }>("/reminders", body),
  updateReminder: (id: number, body: { title: string; note: string; schedule: T.ReminderSchedule; enabled: boolean }) =>
    put<{ reminder: T.Reminder }>(`/reminders/${id}`, body),
  deleteReminder: (id: number) => del<{ ok: boolean }>(`/reminders/${id}`),
  settings: () => get<T.SettingsBundle>("/settings"),
  putSettings: (section: string, value: unknown) => put<{ ok: boolean }>(`/settings/${section}`, value),
  channels: () => get<{ channels: T.ChannelRow[] }>("/channels"),
  putChannel: (name: string, body: { label: string; enabled: boolean; config: T.ChannelConfig }) =>
    put<{ ok: boolean }>(`/channels/${encodeURIComponent(name)}`, body),
  deleteChannel: (name: string) => del<{ ok: boolean }>(`/channels/${encodeURIComponent(name)}`),
  testChannel: (name: string) => post<{ ok: boolean; error: string | null }>(`/channels/${encodeURIComponent(name)}/test`),
  audit: (q: { actor?: string; action?: string; entity_type?: string; entity_id?: string; before?: number; limit?: number }) =>
    get<{ entries: T.AuditEntry[] }>(`/audit${qs(q)}`),
  jobs: (q: { job?: string; limit?: number }) => get<{ jobs: T.JobRun[] }>(`/jobs${qs(q)}`),
  dataStatus: () => get<T.DataStatus>("/data/status"),
  dataSync: (kind: "quotes" | "daily" | "full" | "corporate_actions") => post<{ accepted: boolean }>("/data/sync", { kind }),
  fmpCheck: () => get<{ checks: { endpoint: string; api: string; ok: boolean; detail: string }[] }>("/data/fmp-check"),
};
