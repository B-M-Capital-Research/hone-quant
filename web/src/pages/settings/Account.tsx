import { For, Show, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { Dialog, Pct, toast, toastError } from "@/components/ui";
import { tpl } from "@/i18n";
import { common } from "@/i18n/common";
import { settingsText } from "@/i18n/settings";
import { ApiError, api } from "@/lib/api";
import { onServerEvent } from "@/lib/events";
import { fmtDate, fmtDateTime, fmtMoney, toNumber } from "@/lib/format";
import { isAdmin, refreshMarket } from "@/lib/session";
import type { Account } from "@/lib/types";
import { normalizeNumText } from "./num";
import { Field, Gate, SIcon, createLoader, focusFirstInvalid } from "./shared";

export default function AccountSection() {
  const accounts = createLoader(() => api.accounts());
  const dash = createLoader(() => api.dashboard());
  onMount(() => {
    const off = onServerEvent(["account"], () => {
      void accounts.reload();
      void dash.reload();
    });
    onCleanup(off);
  });
  const reload = async () => {
    await Promise.all([accounts.reload(), dash.reload()]);
    void refreshMarket();
  };
  return (
    <Gate loader={accounts}>
      {(data) => {
        const active = () => data().accounts.find((a) => a.status === "active") ?? null;
        const archived = () => data().accounts.filter((a) => a.status === "archived");
        return (
          <>
            <Show when={active()}>{(a) => <CurrentCard account={a()} nav={dash.value()?.valuation.nav ?? null} ret={dash.value()?.valuation.total_return ?? null} />}</Show>
            <HistoryCard accounts={archived()} />
            <Show when={active()}>{(a) => <DangerZone account={a()} onDone={reload} />}</Show>
          </>
        );
      }}
    </Gate>
  );
}

function CurrentCard(props: { account: Account; nav: number | null; ret: number | null }) {
  const t = settingsText;
  return (
    <div class="card">
      <div class="card-head">
        <div class="head-text">
          <h2>{t().account.current_title}</h2>
          <div class="sub">{t().account.current_sub}</div>
        </div>
        <span class="spacer" />
        <span class="chip orange">
          <SIcon name="shield" size={12} />
          {common().app.paper}
        </span>
      </div>
      <div class="card-body">
        <div class="account-grid">
          <div class="account-stat">
            <span class="label">{t().account.initial_cash}</span>
            <span class="value num">{fmtMoney(props.account.initial_cash)}</span>
          </div>
          <div class="account-stat">
            <span class="label">{t().account.cash}</span>
            <span class="value num">{fmtMoney(props.account.cash)}</span>
          </div>
          <div class="account-stat">
            <span class="label">{t().account.nav}</span>
            <span class="value num">{fmtMoney(props.nav)}</span>
          </div>
          <div class="account-stat">
            <span class="label">{t().account.total_return}</span>
            <span class="value">
              <Pct value={props.ret} />
            </span>
          </div>
        </div>
        <dl class="kv settings-kv" style={{ "margin-top": "18px" }}>
          <dt>{t().account.name}</dt>
          <dd>
            {props.account.name} <span class="muted mono xs">#{props.account.id}</span>
          </dd>
          <dt>{t().account.mode}</dt>
          <dd>{t().account.mode_paper}</dd>
          <dt>{t().account.currency}</dt>
          <dd>{props.account.base_currency}</dd>
          <dt>{t().account.inception}</dt>
          <dd class="num">{fmtDate(props.account.inception_date)}</dd>
          <dt>{t().account.created}</dt>
          <dd class="num">{fmtDateTime(props.account.created_at)}</dd>
        </dl>
      </div>
    </div>
  );
}

function HistoryCard(props: { accounts: Account[] }) {
  const t = settingsText;
  return (
    <div class="card">
      <div class="card-head">
        <div class="head-text">
          <h2>{t().account.history_title}</h2>
          <div class="sub">{t().account.history_sub}</div>
        </div>
      </div>
      <Show when={props.accounts.length > 0} fallback={<p class="card-body muted small">{t().account.history_empty}</p>}>
        <div class="table-wrap">
          <table class="table">
            <thead>
              <tr>
                <th>{t().account.col_id}</th>
                <th>{t().account.name}</th>
                <th class="r">{t().account.initial_cash}</th>
                <th>{t().account.inception}</th>
                <th>{t().account.col_archived}</th>
                <th class="r">{t().account.col_final_cash}</th>
              </tr>
            </thead>
            <tbody>
              <For each={props.accounts}>
                {(a) => (
                  <tr>
                    <td class="mono small">#{a.id}</td>
                    <td>{a.name}</td>
                    <td class="r">{fmtMoney(a.initial_cash)}</td>
                    <td class="num small nowrap">{fmtDate(a.inception_date)}</td>
                    <td class="num small nowrap">{fmtDateTime(a.archived_at)}</td>
                    <td class="r">{fmtMoney(a.cash)}</td>
                  </tr>
                )}
              </For>
            </tbody>
          </table>
        </div>
      </Show>
    </div>
  );
}

function DangerZone(props: { account: Account; onDone: () => Promise<void> }) {
  const t = settingsText;
  const [open, setOpen] = createSignal(false);
  return (
    <div class="card danger-zone">
      <div class="card-head">
        <span class="danger-icon">
          <SIcon name="alert" size={16} />
        </span>
        <div class="head-text">
          <h2>{t().account.danger_title}</h2>
          <div class="sub">{t().account.danger_sub}</div>
        </div>
      </div>
      <div class="card-body danger-body">
        <ul class="danger-points">
          <For each={t().account.danger_points}>{(p) => <li>{p}</li>}</For>
        </ul>
        <div class="danger-action">
          <Show when={isAdmin()} fallback={<p class="muted small">{t().account.viewer_note}</p>}>
            <button type="button" class="btn danger" onClick={() => setOpen(true)}>
              {t().account.reset_button}
            </button>
          </Show>
        </div>
      </div>
      <Show when={open()}>
        <ResetDialog account={props.account} onClose={() => setOpen(false)} onDone={props.onDone} />
      </Show>
    </div>
  );
}

const MIN_CASH = 1_000;
const MAX_CASH = 10_000_000_000;

function ResetDialog(props: { account: Account; onClose: () => void; onDone: () => Promise<void> }) {
  const t = settingsText;
  const c = common;
  const initial = toNumber(props.account.initial_cash) ?? 1_000_000;
  const [cashText, setCashText] = createSignal(initial.toLocaleString("en-US", { maximumFractionDigits: 2 }));
  const [confirmText, setConfirmText] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [touched, setTouched] = createSignal(false);
  const [serverError, setServerError] = createSignal<{ field: "cash" | "confirm" | null; message: string } | null>(null);

  const cash = createMemo(() => {
    const raw = normalizeNumText(cashText());
    if (!/^\d+(\.\d{1,2})?$/.test(raw)) return null;
    const n = Number(raw);
    return n >= MIN_CASH && n <= MAX_CASH ? n : null;
  });
  const cashError = () => {
    if (serverError()?.field === "cash") return serverError()!.message;
    return touched() && cash() === null ? `${settingsText().v.number} · ${t().account.new_cash_hint}` : undefined;
  };
  const confirmOk = () => confirmText().trim() === "RESET";
  const confirmError = () => {
    if (serverError()?.field === "confirm") return serverError()!.message;
    return confirmText() && !confirmOk() && confirmText().length >= 5 ? t().account.confirm_mismatch : undefined;
  };

  const submit = async () => {
    setTouched(true);
    const value = cash();
    if (value === null || !confirmOk()) {
      focusFirstInvalid();
      return;
    }
    setBusy(true);
    setServerError(null);
    try {
      await api.resetAccount(value);
      toast(t().account.reset_done, tpl(t().account.reset_done_body, { cash: fmtMoney(value) }), "success");
      props.onClose();
      await props.onDone();
    } catch (error) {
      if (error instanceof ApiError && error.status === 400) {
        setServerError({ field: /RESET/.test(error.message) ? "confirm" : /cash/i.test(error.message) ? "cash" : null, message: error.message });
      } else if (error instanceof ApiError && error.status === 409) {
        setServerError({ field: null, message: error.message });
      }
      toastError(error);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      title={t().account.dialog_title}
      onClose={() => !busy() && props.onClose()}
      footer={
        <>
          <button type="button" class="btn" onClick={() => props.onClose()} disabled={busy()}>
            {c().actions.cancel}
          </button>
          <button type="button" class="btn danger solid" disabled={busy() || cash() === null || !confirmOk()} onClick={() => void submit()}>
            {busy() ? t().account.resetting : t().account.reset_confirm}
          </button>
        </>
      }
    >
      {/* A destructive action: Enter never submits, only the explicit button does. */}
      <form class="stack" style={{ gap: "16px" }} novalidate onSubmit={(e) => e.preventDefault()}>
        <div class="callout critical">
          <SIcon name="alert" size={16} />
          <span>{t().account.dialog_warning}</span>
        </div>
        <ul class="danger-points compact">
          <For each={t().account.danger_points}>{(p) => <li>{p}</li>}</For>
        </ul>
        <Field id="reset-cash" label={t().account.new_cash} hint={t().account.new_cash_hint} error={cashError()}>
          <div class="input-group num-group has-prefix wide">
            <span class="addon pre">$</span>
            <input
              id="reset-cash"
              class="input num"
              classList={{ invalid: !!cashError() }}
              aria-invalid={cashError() ? "true" : "false"}
              inputmode="decimal"
              autocomplete="off"
              value={cashText()}
              onInput={(e) => {
                setCashText(e.currentTarget.value);
                setServerError(null);
              }}
              onBlur={() => setTouched(true)}
            />
            <span class="addon">{props.account.base_currency}</span>
          </div>
        </Field>
        <Field id="reset-confirm" label={t().account.confirm_label} error={confirmError()}>
          <input
            id="reset-confirm"
            class="input mono"
            classList={{ invalid: !!confirmError() }}
            aria-invalid={confirmError() ? "true" : "false"}
            autocomplete="off"
            autocapitalize="characters"
            spellcheck={false}
            placeholder="RESET"
            value={confirmText()}
            onInput={(e) => {
              setConfirmText(e.currentTarget.value);
              setServerError(null);
            }}
          />
        </Field>
        <Show when={serverError() && serverError()!.field === null}>
          <div class="callout critical" role="alert">
            <SIcon name="alert" size={16} />
            <span>{serverError()!.message}</span>
          </div>
        </Show>
        <p class="muted xs">{c().confirm.irreversible}</p>
      </form>
    </Dialog>
  );
}
