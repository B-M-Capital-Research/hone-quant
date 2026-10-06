import { For, type JSX, type ParentProps, Show, createSignal, onCleanup, onMount } from "solid-js";
import { Portal } from "solid-js/web";
import { createStore } from "solid-js/store";
import { common } from "@/i18n/common";
import { ApiError } from "@/lib/api";
import { fmtMoney, fmtPct, moneyPolarity, polarity } from "@/lib/format";
import type { OrderStatus, PlanStatus, Severity, Side } from "@/lib/types";
import { Icon, type IconName } from "./Icon";

// ---------------------------------------------------------------------------------------------
// Toasts
// ---------------------------------------------------------------------------------------------

interface Toast {
  id: number;
  title: string;
  body?: string;
  severity: Severity;
}

const [toasts, setToasts] = createStore<Toast[]>([]);
let toastId = 0;

export function toast(title: string, body?: string, severity: Severity = "info", ttl = 4200) {
  const id = ++toastId;
  setToasts((list) => [...list.slice(-3), { id, title, body, severity }]);
  setTimeout(() => setToasts((list) => list.filter((t) => t.id !== id)), ttl);
}

export function toastError(error: unknown, fallback?: string) {
  const c = common();
  if (error instanceof ApiError) {
    if (error.code === "network") return toast(c.states.error, c.states.network, "critical");
    if (error.status === 403 && error.message.includes("admin")) return toast(c.states.no_permission, undefined, "warning");
    const detail = error.fields?.length ? error.fields.map((f) => `${f.path}: ${f.message}`).join("\n") : error.message;
    return toast(fallback ?? c.states.error, detail, "critical", 7000);
  }
  toast(fallback ?? c.states.error, String(error), "critical", 7000);
}

const SEVERITY_ICON: Record<Severity, IconName> = { info: "info", success: "check", warning: "alert", critical: "alert" };

export function Toasts() {
  return (
    <Portal>
      <div class="toasts" role="status" aria-live="polite">
        <For each={toasts}>
          {(t) => (
            <div class={`toast ${t.severity}`}>
              <Icon name={SEVERITY_ICON[t.severity]} size={18} />
              <div style={{ "min-width": 0 }}>
                <div class="title">{t.title}</div>
                <Show when={t.body}>
                  <div class="body">{t.body}</div>
                </Show>
              </div>
            </div>
          )}
        </For>
      </div>
    </Portal>
  );
}

// ---------------------------------------------------------------------------------------------
// Dialogs
// ---------------------------------------------------------------------------------------------

export function Dialog(
  props: ParentProps<{ title: string; onClose: () => void; wide?: boolean; footer?: JSX.Element; subtitle?: string }>,
) {
  const onKey = (event: KeyboardEvent) => {
    if (event.key === "Escape") props.onClose();
  };
  onMount(() => {
    document.addEventListener("keydown", onKey);
    document.body.style.overflow = "hidden";
  });
  onCleanup(() => {
    document.removeEventListener("keydown", onKey);
    document.body.style.overflow = "";
  });
  return (
    <Portal>
      <div class="backdrop" onClick={(e) => e.target === e.currentTarget && props.onClose()}>
        <div class={`dialog ${props.wide ? "wide" : ""}`} role="dialog" aria-modal="true" aria-label={props.title}>
          <div class="dialog-head">
            <div style={{ flex: 1, "min-width": 0 }}>
              <h2>{props.title}</h2>
              <Show when={props.subtitle}>
                <p class="muted small" style={{ "margin-top": "4px" }}>
                  {props.subtitle}
                </p>
              </Show>
            </div>
            <button class="btn ghost icon sm" onClick={props.onClose} aria-label={common().actions.close}>
              <Icon name="x" size={16} />
            </button>
          </div>
          <div class="dialog-body">{props.children}</div>
          <Show when={props.footer}>
            <div class="dialog-foot">{props.footer}</div>
          </Show>
        </div>
      </div>
    </Portal>
  );
}

/**
 * Confirmation with an optional reason field. Resolves to the reason (possibly "") on confirm,
 * or null when dismissed.
 */
const [confirmState, setConfirmState] = createSignal<{
  title: string;
  body: string;
  confirmLabel: string;
  danger: boolean;
  askReason: boolean;
  reasonLabel: string;
  resolve: (value: string | null) => void;
} | null>(null);

export function confirmAction(opts: {
  title: string;
  body: string;
  confirmLabel?: string;
  danger?: boolean;
  askReason?: boolean;
  reasonLabel?: string;
}): Promise<string | null> {
  return new Promise((resolve) => {
    setConfirmState({
      title: opts.title,
      body: opts.body,
      confirmLabel: opts.confirmLabel ?? common().actions.confirm,
      danger: opts.danger ?? false,
      askReason: opts.askReason ?? false,
      reasonLabel: opts.reasonLabel ?? common().words.reason,
      resolve,
    });
  });
}

export function ConfirmHost() {
  const [reason, setReason] = createSignal("");
  const close = (value: string | null) => {
    confirmState()?.resolve(value);
    setConfirmState(null);
    setReason("");
  };
  return (
    <Show when={confirmState()}>
      {(state) => (
        <Dialog
          title={state().title}
          onClose={() => close(null)}
          footer={
            <>
              <button class="btn" onClick={() => close(null)}>
                {common().actions.cancel}
              </button>
              <button class={`btn ${state().danger ? "danger" : "primary"}`} onClick={() => close(reason())}>
                {state().confirmLabel}
              </button>
            </>
          }
        >
          <div class="stack" style={{ gap: "14px" }}>
            <p style={{ "white-space": "pre-line" }}>{state().body}</p>
            <Show when={state().askReason}>
              <div class="field">
                <label for="confirm-reason">{state().reasonLabel}</label>
                <textarea
                  id="confirm-reason"
                  class="textarea"
                  rows={2}
                  value={reason()}
                  onInput={(e) => setReason(e.currentTarget.value)}
                />
              </div>
            </Show>
            <p class="muted xs">{common().confirm.irreversible}</p>
          </div>
        </Dialog>
      )}
    </Show>
  );
}

// ---------------------------------------------------------------------------------------------
// Chips & values
// ---------------------------------------------------------------------------------------------

const PLAN_TONE: Record<string, string> = {
  pending: "orange",
  executing: "blue",
  executed: "green",
  partially_executed: "yellow",
  no_action: "",
  cancelled: "",
  expired: "yellow",
  skipped: "",
  failed: "red",
  scheduled: "outline",
  cancelled_ahead: "",
};

export function PlanStatusChip(props: { status: PlanStatus | "scheduled" | "cancelled_ahead" }) {
  return (
    <span class={`chip ${PLAN_TONE[props.status] ?? ""}`}>
      <span class="dot" />
      {common().plan_status[props.status]}
    </span>
  );
}

const ORDER_TONE: Record<OrderStatus, string> = {
  planned: "orange",
  skipped: "",
  filled: "green",
  partially_filled: "yellow",
  rejected: "red",
  cancelled: "",
  expired: "yellow",
};

export function OrderStatusChip(props: { status: OrderStatus; reason?: string | null }) {
  const reasonText = () => {
    const r = props.reason as keyof ReturnType<typeof common>["order_status_reason"] | undefined;
    return r ? common().order_status_reason[r] ?? r : null;
  };
  return (
    <span class={`chip ${ORDER_TONE[props.status]}`} title={reasonText() ?? undefined}>
      {common().order_status[props.status]}
      <Show when={reasonText() && props.status !== "filled"}>
        <span style={{ opacity: 0.8, "font-weight": 500 }}>· {reasonText()}</span>
      </Show>
    </span>
  );
}

export function SideChip(props: { side: Side }) {
  return (
    <span class={`chip side-${props.side}`}>
      <Icon name={props.side === "buy" ? "arrow_up" : "arrow_down"} size={11} />
      {common().side[props.side]}
    </span>
  );
}

/** Signed percentage coloured by polarity (sign is always shown, so colour is never alone). */
export function Pct(props: { value: number | null | undefined; dp?: number; class?: string }) {
  return <span class={`num ${polarity(props.value, props.dp ?? 2)} ${props.class ?? ""}`}>{fmtPct(props.value, { dp: props.dp ?? 2, sign: true })}</span>;
}

export function Money(props: { value: number | string | null | undefined; signed?: boolean; compact?: boolean; dp?: number; class?: string }) {
  const tone = () => (props.signed ? moneyPolarity(props.value, props.dp ?? 2) : "");
  return <span class={`num ${tone()} ${props.class ?? ""}`}>{fmtMoney(props.value, { sign: props.signed, compact: props.compact, dp: props.dp })}</span>;
}

// ---------------------------------------------------------------------------------------------
// States
// ---------------------------------------------------------------------------------------------

export function Loading(props: { label?: string }) {
  return (
    <div class="loading-row">
      <div class="spinner" />
      {props.label ?? common().states.loading}
    </div>
  );
}

export function ErrorState(props: { error: unknown; onRetry?: () => void }) {
  const message = () => {
    const e = props.error;
    if (e instanceof ApiError) return e.code === "network" ? common().states.network : e.message;
    return String(e ?? "");
  };
  return (
    <div class="empty">
      <Icon name="alert" size={22} />
      <div class="title">{common().states.error}</div>
      <div>{message()}</div>
      <Show when={props.onRetry}>
        <button class="btn sm" onClick={() => props.onRetry?.()}>
          {common().actions.retry}
        </button>
      </Show>
    </div>
  );
}

export function Empty(props: ParentProps<{ title?: string; icon?: IconName }>) {
  return (
    <div class="empty">
      <Show when={props.icon}>
        <Icon name={props.icon!} size={22} />
      </Show>
      <div class="title">{props.title ?? common().states.empty}</div>
      {props.children}
    </div>
  );
}

// ---------------------------------------------------------------------------------------------
// Controls
// ---------------------------------------------------------------------------------------------

export function Segmented<T extends string>(props: {
  value: T;
  options: { value: T; label: string; title?: string }[];
  onChange: (value: T) => void;
  label?: string;
}) {
  return (
    <div class="segmented" role="group" aria-label={props.label}>
      <For each={props.options}>
        {(option) => (
          <button
            type="button"
            aria-pressed={props.value === option.value}
            title={option.title}
            onClick={() => props.onChange(option.value)}
          >
            {option.label}
          </button>
        )}
      </For>
    </div>
  );
}

export function Switch(props: { checked: boolean; onChange: (value: boolean) => void; label?: JSX.Element; disabled?: boolean }) {
  return (
    <label class="switch" style={{ opacity: props.disabled ? 0.5 : 1 }}>
      <input
        type="checkbox"
        checked={props.checked}
        disabled={props.disabled}
        onChange={(e) => props.onChange(e.currentTarget.checked)}
      />
      <span class="track" />
      <Show when={props.label}>
        <span>{props.label}</span>
      </Show>
    </label>
  );
}

/** Click-outside popover anchored below its trigger. */
export function Popover(props: ParentProps<{ trigger: (open: () => void) => JSX.Element; align?: "left" | "right"; width?: number }>) {
  const [open, setOpen] = createSignal(false);
  let root!: HTMLDivElement;
  const onDoc = (event: MouseEvent) => {
    if (open() && !root.contains(event.target as Node)) setOpen(false);
  };
  const onKey = (event: KeyboardEvent) => {
    if (event.key === "Escape") setOpen(false);
  };
  onMount(() => {
    document.addEventListener("mousedown", onDoc);
    document.addEventListener("keydown", onKey);
  });
  onCleanup(() => {
    document.removeEventListener("mousedown", onDoc);
    document.removeEventListener("keydown", onKey);
  });
  return (
    <div ref={root} style={{ position: "relative" }}>
      {props.trigger(() => setOpen((v) => !v))}
      <Show when={open()}>
        <div
          class="popover"
          style={{
            top: "calc(100% + 8px)",
            [props.align === "left" ? "left" : "right"]: "0",
            width: props.width ? `${props.width}px` : undefined,
          }}
          onClick={(e) => {
            if ((e.target as HTMLElement).closest("[data-close]")) setOpen(false);
          }}
        >
          {props.children}
        </div>
      </Show>
    </div>
  );
}

export function Kpi(props: { label: string; value: JSX.Element; delta?: JSX.Element; hero?: boolean; title?: string }) {
  return (
    <div class="kpi" title={props.title}>
      <span class="label">{props.label}</span>
      <span class={`value ${props.hero ? "hero" : ""}`}>{props.value}</span>
      <Show when={props.delta}>
        <span class="delta">{props.delta}</span>
      </Show>
    </div>
  );
}

/** Weight bar with the target as a tick. */
export function WeightBar(props: { weight: number; target?: number | null; max?: number }) {
  const max = () => Math.max(props.max ?? 0.1, props.weight, props.target ?? 0, 0.0001);
  return (
    <div class="weightbar" title={props.target != null ? `${fmtPct(props.weight, { dp: 1 })} → ${fmtPct(props.target, { dp: 1 })}` : undefined}>
      <div class="fill" style={{ width: `${Math.min(100, (props.weight / max()) * 100)}%` }} />
      <Show when={props.target != null}>
        <div class="target" style={{ left: `calc(${Math.min(100, ((props.target ?? 0) / max()) * 100)}% - 1px)` }} />
      </Show>
    </div>
  );
}

/** Downloads rows as CSV (UTF-8 with BOM so Excel opens Chinese text correctly). */
export function downloadCsv(filename: string, header: string[], rows: (string | number | null | undefined)[][]) {
  const escape = (v: string | number | null | undefined) => {
    const s = v === null || v === undefined ? "" : String(v);
    return /[",\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
  };
  const csv = "﻿" + [header, ...rows].map((r) => r.map(escape).join(",")).join("\n");
  const url = URL.createObjectURL(new Blob([csv], { type: "text/csv;charset=utf-8" }));
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}
