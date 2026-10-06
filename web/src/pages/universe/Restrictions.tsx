import { For, Show, createMemo, createSignal } from "solid-js";
import { Icon } from "@/components/Icon";
import { Dialog, Empty, confirmAction, toast, toastError } from "@/components/ui";
import { locale, tpl } from "@/i18n";
import { common } from "@/i18n/common";
import { universeText } from "@/i18n/universe";
import { api } from "@/lib/api";
import { DASH, fmtDateTime, fmtQty, fmtWeight } from "@/lib/format";
import { isAdmin } from "@/lib/session";
import type { Restriction, UniverseView } from "@/lib/types";
import { RestrictionChip } from "./Members";
import { type Weights, assetOf, modeText, pickText, restrictionState, stateText } from "./helpers";
import { actorName } from "@/lib/names";

function CompanyCell(props: { view: UniverseView; symbol: string }) {
  const asset = () => assetOf(props.view, props.symbol);
  return (
    <div class="name-cell">
      <span class="ticker">{props.symbol}</span>
      <span class="name">{asset() ? pickText(asset()!, "name") : ""}</span>
    </div>
  );
}

function periodText(r: Restriction): string {
  const t = universeText().restrictions;
  return tpl(t.period, { from: r.starts_on, to: r.ends_on ?? t.open_ended });
}

export function ModeExplainer() {
  const t = universeText;
  return (
    <div class="uv-modes">
      <div class="uv-mode">
        <span class="chip red">
          <Icon name="ban" size={11} /> {t().restrictions.mode_exclude}
        </span>
        <p>{t().restrictions.exclude_body}</p>
      </div>
      <div class="uv-mode">
        <span class="chip yellow">
          <Icon name="lock" size={11} /> {t().restrictions.mode_lock}
        </span>
        <p>{t().restrictions.lock_body}</p>
      </div>
    </div>
  );
}

export function ActiveRestrictions(props: { view: UniverseView; active: Restriction[]; onChanged: () => void; onAdd: () => void }) {
  const t = universeText;
  const revoke = async (r: Restriction) => {
    const ok = await confirmAction({
      title: t().restrictions.revoke_title,
      body: tpl(t().restrictions.revoke_body, { symbol: r.symbol, mode: modeText(r.mode) }),
      confirmLabel: t().actions.revoke,
      danger: true,
    });
    if (ok === null) return;
    try {
      await api.revokeRestriction(r.id);
      toast(tpl(t().restrictions.revoked, { symbol: r.symbol }), undefined, "success");
      props.onChanged();
    } catch (error) {
      toastError(error);
    }
  };
  return (
    <Show
      when={props.active.length}
      fallback={
        <Empty title={t().restrictions.empty} icon="shield">
          <span>{t().restrictions.empty_hint}</span>
          <Show when={isAdmin()}>
            <button class="btn sm" onClick={() => props.onAdd()}>
              <Icon name="plus" size={13} /> {t().actions.add_restriction}
            </button>
          </Show>
        </Empty>
      }
    >
      <div class="table-wrap">
        <table class="table">
          <thead>
            <tr>
              <th>{t().restrictions.h_company}</th>
              <th>{t().restrictions.h_mode}</th>
              <th>{t().restrictions.h_reason}</th>
              <th>{t().restrictions.h_period}</th>
              <th>{t().restrictions.h_created}</th>
              <Show when={isAdmin()}>
                <th class="r" />
              </Show>
            </tr>
          </thead>
          <tbody>
            <For each={props.active}>
              {(r) => (
                <tr>
                  <td>
                    <CompanyCell view={props.view} symbol={r.symbol} />
                  </td>
                  <td>
                    <RestrictionChip restriction={r} />
                  </td>
                  <td class="uv-reason">{r.reason || <span class="muted">{DASH}</span>}</td>
                  <td class="nowrap small num">{periodText(r)}</td>
                  <td class="nowrap">
                    <div class="small num">{fmtDateTime(r.created_at)}</div>
                    <div class="xs muted">{actorName(r.created_by)}</div>
                  </td>
                  <Show when={isAdmin()}>
                    <td class="r">
                      <button class="btn sm danger" onClick={() => void revoke(r)}>
                        {t().actions.revoke}
                      </button>
                    </td>
                  </Show>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>
    </Show>
  );
}

export function RestrictionHistory(props: { view: UniverseView; history: Restriction[]; today: string }) {
  const t = universeText;
  const rows = createMemo(() => props.history.filter((r) => restrictionState(r, props.today) !== "active"));
  return (
    <Show when={rows().length} fallback={<Empty title={t().restrictions.history_empty} icon="history" />}>
      <div class="table-wrap">
        <table class="table compact">
          <thead>
            <tr>
              <th>{t().restrictions.h_company}</th>
              <th>{t().restrictions.h_mode}</th>
              <th>{t().restrictions.h_reason}</th>
              <th>{t().restrictions.h_period}</th>
              <th>{t().restrictions.h_state}</th>
              <th>{t().restrictions.h_created}</th>
            </tr>
          </thead>
          <tbody>
            <For each={rows()}>
              {(r) => {
                const state = () => restrictionState(r, props.today);
                return (
                  <tr>
                    <td>
                      <CompanyCell view={props.view} symbol={r.symbol} />
                    </td>
                    <td>
                      <span class="chip outline">
                        <Icon name={r.mode === "exclude" ? "ban" : "lock"} size={11} /> {modeText(r.mode)}
                      </span>
                    </td>
                    <td class="uv-reason">{r.reason || <span class="muted">{DASH}</span>}</td>
                    <td class="nowrap small num">{periodText(r)}</td>
                    <td class="nowrap">
                      <span class={`chip ${state() === "scheduled" ? "blue" : ""}`}>{stateText(state())}</span>
                      <Show when={state() === "revoked"}>
                        <div class="xs muted" style={{ "margin-top": "3px" }}>
                          {r.revoked_by} · {fmtDateTime(r.revoked_at)}
                        </div>
                      </Show>
                    </td>
                    <td class="nowrap">
                      <div class="small num">{fmtDateTime(r.created_at)}</div>
                      <div class="xs muted">{actorName(r.created_by)}</div>
                    </td>
                  </tr>
                );
              }}
            </For>
          </tbody>
        </table>
      </div>
    </Show>
  );
}

export function AddRestrictionDialog(props: {
  view: UniverseView;
  weights: Weights;
  active: Restriction[];
  today: string;
  initialSymbol: string;
  onClose: () => void;
  onAdded: () => void;
}) {
  const t = universeText;
  const c = common;
  const [symbol, setSymbol] = createSignal(props.initialSymbol);
  const [mode, setMode] = createSignal<"exclude" | "lock">("exclude");
  const [reason, setReason] = createSignal("");
  const [endsOn, setEndsOn] = createSignal("");
  const [touched, setTouched] = createSignal(false);
  const [busy, setBusy] = createSignal(false);

  const sectors = () => [...props.view.sectors].sort((a, b) => a.sort_order - b.sort_order);
  const duplicate = () => (symbol() ? props.active.find((r) => r.symbol === symbol()) : undefined);
  const endError = () => (endsOn() && endsOn() < props.today ? tpl(t().add.end_before_start, { date: props.today }) : "");
  const reasonError = () => (touched() && !reason().trim() ? t().add.reason_required : "");
  const canSubmit = () => !!symbol() && !!reason().trim() && !duplicate() && !endError() && !busy();

  const effect = () => {
    const s = symbol();
    if (!s) return "";
    const p = props.weights.position(s);
    if (mode() === "exclude") {
      return p ? tpl(t().add.effect_exclude_held, { symbol: s, qty: fmtQty(p.qty), weight: fmtWeight(p.weight, 2) }) : tpl(t().add.effect_exclude_none, { symbol: s });
    }
    return p ? tpl(t().add.effect_lock_held, { symbol: s, qty: fmtQty(p.qty), weight: fmtWeight(p.weight, 2) }) : tpl(t().add.effect_lock_none, { symbol: s });
  };

  const submit = async () => {
    setTouched(true);
    if (!canSubmit()) return;
    setBusy(true);
    try {
      await api.addRestriction({ symbol: symbol(), mode: mode(), reason: reason().trim(), ends_on: endsOn() || null });
      toast(tpl(t().add.added, { symbol: symbol() }), effect(), "success", 6000);
      props.onAdded();
    } catch (error) {
      toastError(error);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      title={t().add.title}
      onClose={props.onClose}
      footer={
        <>
          <button class="btn" onClick={props.onClose}>
            {c().actions.cancel}
          </button>
          <button class={`btn ${mode() === "exclude" ? "danger" : "primary"}`} onClick={() => void submit()} disabled={busy() || !!duplicate() || !symbol()}>
            {busy() ? c().actions.saving : t().add.submit}
          </button>
        </>
      }
    >
      <div class="stack" style={{ gap: "16px" }}>
        <div class="field">
          <label for="uv-add-symbol">{t().add.company}</label>
          <select id="uv-add-symbol" class="select" value={symbol()} onChange={(e) => setSymbol(e.currentTarget.value)}>
            <option value="" disabled>
              {t().add.pick}
            </option>
            <For each={sectors()}>
              {(sector) => (
                <optgroup label={pickText(sector, "name")}>
                  <For each={props.view.assets.filter((a) => a.sector_id === sector.id).sort((a, b) => a.sort_order - b.sort_order)}>
                    {(a) => (
                      <option value={a.symbol}>
                        {a.symbol} · {pickText(a, "name")}
                      </option>
                    )}
                  </For>
                </optgroup>
              )}
            </For>
          </select>
          <Show when={duplicate()}>
            <span class="error-text">{tpl(t().add.duplicate, { symbol: symbol() })}</span>
          </Show>
        </div>

        <fieldset class="uv-mode-pick">
          <legend class="field-label">{t().add.mode}</legend>
          <div class="uv-mode-grid">
          <label class="uv-mode-option" classList={{ selected: mode() === "exclude" }}>
            <input type="radio" name="uv-mode" value="exclude" checked={mode() === "exclude"} onChange={() => setMode("exclude")} />
            <span>
              <span class="uv-mode-title">
                <Icon name="ban" size={14} /> {t().restrictions.mode_exclude}
              </span>
              <span class="uv-mode-body">{t().restrictions.exclude_body}</span>
            </span>
          </label>
          <label class="uv-mode-option" classList={{ selected: mode() === "lock" }}>
            <input type="radio" name="uv-mode" value="lock" checked={mode() === "lock"} onChange={() => setMode("lock")} />
            <span>
              <span class="uv-mode-title">
                <Icon name="lock" size={14} /> {t().restrictions.mode_lock}
              </span>
              <span class="uv-mode-body">{t().restrictions.lock_body}</span>
            </span>
          </label>
          </div>
        </fieldset>

        <div class="field">
          <label for="uv-add-reason">{t().add.reason}</label>
          <textarea
            id="uv-add-reason"
            class="textarea"
            rows={2}
            placeholder={t().add.reason_placeholder}
            value={reason()}
            onInput={(e) => setReason(e.currentTarget.value)}
            onBlur={() => setTouched(true)}
          />
          <Show when={reasonError()}>
            <span class="error-text">{reasonError()}</span>
          </Show>
        </div>

        <div class="field">
          <label for="uv-add-ends">{t().add.ends_on}</label>
          <input id="uv-add-ends" class="input uv-date" type="date" min={props.today} value={endsOn()} onInput={(e) => setEndsOn(e.currentTarget.value)} />
          <span class="hint">{t().add.ends_hint}</span>
          <Show when={endError()}>
            <span class="error-text">{endError()}</span>
          </Show>
        </div>

        <Show when={symbol() && !duplicate()}>
          <div class={`callout ${mode() === "exclude" ? "warn" : "info"}`}>
            <Icon name={mode() === "exclude" ? "alert" : "info"} size={16} />
            <div class="stack" style={{ gap: "4px" }}>
              <span>
                <strong>{effect()}</strong>
              </span>
              <span class="xs">
                {tpl(t().add.starts, { date: props.today })}
                {locale() === "zh" ? "" : " "}
                {t().add.pending_note}
              </span>
            </div>
          </div>
        </Show>
      </div>
    </Dialog>
  );
}
