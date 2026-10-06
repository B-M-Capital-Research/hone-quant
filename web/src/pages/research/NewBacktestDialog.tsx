import { For, Show, createMemo, createSignal, onMount } from "solid-js";
import { createStore } from "solid-js/store";
import { Icon } from "@/components/Icon";
import { Dialog, Segmented, toast } from "@/components/ui";
import { tpl } from "@/i18n";
import { common } from "@/i18n/common";
import { researchText } from "@/i18n/research";
import { ApiError, api } from "@/lib/api";
import { fmtDate, fmtPct } from "@/lib/format";
import type { BacktestRow, CostModel, DataStatus, SettingsBundle, StrategyOverview, StrategyParams, UniverseView } from "@/lib/types";
import {
  COST_FIELDS,
  type CostKey,
  UNIVERSE_EW,
  addDays,
  addYears,
  benchLong,
  costToDisplay,
  costsEqual,
  costsSummary,
  daysBetween,
  dataStatusRef,
  dataWindow,
  fmtYears,
  jsonEqual,
  marketTodayServer,
  maxLookback,
  parseDay,
  pickText,
  resolveStrategy,
  settingsRef,
  strategyRef,
  universeRef,
  warmupCalendarDays,
  yearsBetween,
} from "./model";
import { actorName, strategyName as versionName } from "@/lib/names";

export type Prefill =
  | { kind: "default" }
  | { kind: "version"; versionId: number }
  | { kind: "preset"; presetId: string }
  | { kind: "duplicate"; row: BacktestRow };

type Source = "active" | "version" | "preset" | "custom";
type RangeMode = "1Y" | "3Y" | "5Y" | "MAX" | "custom";
type SlotChoice = "open" | "close" | "both";
type FieldKey = "name" | "strategy" | "dates" | "start" | "end" | "cash" | "rf" | CostKey;

interface FormState {
  name: string;
  nameTouched: boolean;
  source: Source;
  versionId: number | null;
  presetId: string;
  customParams: StrategyParams | null;
  range: RangeMode;
  start: string;
  end: string;
  cash: string;
  slots: SlotChoice;
  frequency: "daily" | "weekly" | "monthly";
  benchmark: string;
  rf: string;
  costs: Record<CostKey, string>;
  costsOpen: boolean;
}

const MAX_YEARS_DAYS = 366 * 25;

export function NewBacktestDialog(props: { prefill: Prefill; onClose: () => void; onCreated: (row: BacktestRow) => void }) {
  const r = researchText;
  const c = common;
  const [ready, setReady] = createSignal(false);
  const [loadError, setLoadError] = createSignal<unknown>(null);
  const [overview, setOverview] = createSignal<StrategyOverview>();
  const [settings, setSettings] = createSignal<SettingsBundle>();
  const [universe, setUniverse] = createSignal<UniverseView>();
  const [status, setStatus] = createSignal<DataStatus>();
  const [notice, setNotice] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [submitted, setSubmitted] = createSignal(false);
  const [serverErrors, setServerErrors] = createSignal<Partial<Record<FieldKey, string>>>({});
  let formRef: HTMLFormElement | undefined;
  const [generalError, setGeneralError] = createSignal<string | null>(null);
  const today = marketTodayServer();

  const [form, setForm] = createStore<FormState>({
    name: "",
    nameTouched: false,
    source: "active",
    versionId: null,
    presetId: "",
    customParams: null,
    range: "5Y",
    start: addYears(today, -5),
    end: today,
    cash: "1000000",
    slots: "both",
    frequency: "daily",
    benchmark: "",
    rf: "0",
    costs: costToDisplay({ commission_per_share: 0.005, commission_min: 1, commission_max_rate: 0.01, commission_rate: 0, slippage_bps: 5, sell_fee_rate: 0.0000278 }),
    costsOpen: false,
  });

  onMount(async () => {
    try {
      const [ov, st, un] = await Promise.all([strategyRef.get(), settingsRef.get(), universeRef.get()]);
      setOverview(ov);
      setSettings(st);
      setUniverse(un);
      initialise(ov, st);
      setReady(true);
    } catch (error) {
      setLoadError(error);
    }
    // Coverage only refines the "max" quick pick and the warm-up hint.
    dataStatusRef
      .get()
      .then((s) => {
        setStatus(s);
        if (form.range === "MAX") applyRange("MAX");
      })
      .catch(() => undefined);
  });

  function initialise(ov: StrategyOverview, st: SettingsBundle) {
    const p = props.prefill;
    const defaults = {
      benchmark: st.benchmarks.primary,
      rf: String(+(st.benchmarks.risk_free_rate * 100).toPrecision(8)),
      costs: costToDisplay(st.execution.costs),
    };
    const defaultPreset = ov.presets[0]?.id ?? "";
    let source: Source = ov.active ? "active" : "preset";
    let versionId: number | null = ov.active?.id ?? ov.versions[0]?.id ?? null;
    let presetId = defaultPreset;
    let customParams: StrategyParams | null = null;
    if (p.kind === "version") {
      const version = ov.versions.find((v) => v.id === p.versionId);
      if (version) {
        source = ov.active?.id === version.id ? "active" : "version";
        versionId = version.id;
      } else {
        setNotice(tpl(r().source.version_missing, { id: p.versionId }));
      }
    } else if (p.kind === "preset") {
      if (ov.presets.some((x) => x.id === p.presetId)) {
        source = "preset";
        presetId = p.presetId;
      } else {
        setNotice(tpl(ov.active ? r().source.preset_missing_active : r().source.preset_missing, { id: p.presetId }));
      }
    }
    if (p.kind === "duplicate") {
      const row = p.row;
      const ref = resolveStrategy(row, ov);
      if (ref.kind === "version" && ov.versions.some((v) => v.id === ref.id)) {
        source = ov.active?.id === ref.id ? "active" : "version";
        versionId = ref.id;
      } else if (ref.kind === "preset") {
        source = "preset";
        presetId = ref.id;
      } else {
        source = "custom";
        customParams = row.config.params;
      }
      const end = row.config.end > today ? today : row.config.end;
      setForm({
        name: `${row.name}${r().form.copy_suffix}`.slice(0, 120),
        nameTouched: true,
        source,
        versionId,
        presetId,
        customParams,
        range: "custom",
        start: row.config.start,
        end,
        cash: String(row.config.initial_cash),
        slots: row.config.slots.length > 1 ? "both" : (row.config.slots[0] ?? "open"),
        frequency: row.config.frequency,
        benchmark: row.config.benchmark ?? defaults.benchmark,
        rf: String(+(row.config.risk_free_rate * 100).toPrecision(8)),
        costs: costToDisplay(row.config.costs),
        costsOpen: !costsEqual(row.config.costs, st.execution.costs),
      });
      return;
    }
    setForm({ source, versionId, presetId, customParams, benchmark: defaults.benchmark, rf: defaults.rf, costs: defaults.costs });
    applyRange("5Y");
  }

  // ---- Strategy -------------------------------------------------------------------------------

  const selectedParams = createMemo<StrategyParams | null>(() => {
    const ov = overview();
    if (!ov) return null;
    switch (form.source) {
      case "active":
        return ov.active?.params ?? null;
      case "version":
        return ov.versions.find((v) => v.id === form.versionId)?.params ?? null;
      case "preset":
        return ov.presets.find((p) => p.id === form.presetId)?.params ?? null;
      case "custom":
        return form.customParams;
    }
  });

  const strategyName = createMemo(() => {
    const ov = overview();
    if (!ov) return "";
    switch (form.source) {
      case "active":
        return ov.active?.name ?? "";
      case "version":
        return ov.versions.find((v) => v.id === form.versionId)?.name ?? "";
      case "preset": {
        const preset = ov.presets.find((p) => p.id === form.presetId);
        return preset ? pickText(preset, "name") : "";
      }
      case "custom":
        return r().source.custom;
    }
  });

  /** Presets whose parameters equal the selected version, to explain where it came from. */
  const versionBasis = createMemo(() => {
    const ov = overview();
    const params = selectedParams();
    if (!ov || !params || form.source === "preset" || form.source === "custom") return null;
    const preset = ov.presets.find((p) => jsonEqual(p.params, params));
    return preset ? pickText(preset, "name") : null;
  });

  const sourceOptions = () => {
    const opts: { value: Source; label: string }[] = [];
    if (overview()?.active) opts.push({ value: "active", label: r().source.active });
    if ((overview()?.versions.length ?? 0) > 0) opts.push({ value: "version", label: r().source.version });
    opts.push({ value: "preset", label: r().source.preset });
    if (form.customParams) opts.push({ value: "custom", label: r().source.custom });
    return opts;
  };

  // ---- Dates ----------------------------------------------------------------------------------

  const coverage = createMemo(() => dataWindow(status(), universe()));
  const earliest = createMemo(() => {
    const w = coverage();
    const params = selectedParams();
    if (!w || !params) return null;
    return addDays(w.first, warmupCalendarDays(params));
  });

  function applyRange(mode: RangeMode) {
    if (mode === "custom") {
      setForm("range", "custom");
      return;
    }
    const end = today;
    let start = end;
    if (mode === "1Y") start = addYears(end, -1);
    if (mode === "3Y") start = addYears(end, -3);
    if (mode === "5Y") start = addYears(end, -5);
    if (mode === "MAX") {
      const e = earliest();
      start = e && e < end ? e : addYears(end, -10);
      // The server limits a run to 25 years.
      if (daysBetween(start, end) > MAX_YEARS_DAYS) start = addDays(end, -MAX_YEARS_DAYS);
    }
    setForm({ range: mode, start, end });
  }

  const rangeLabel = () => {
    switch (form.range) {
      case "1Y":
        return r().range.y1;
      case "3Y":
        return r().range.y3;
      case "5Y":
        return r().range.y5;
      case "MAX":
        return r().range.max;
      default:
        return `${form.start} → ${form.end}`;
    }
  };

  // Generated name until the user types their own.
  const autoName = createMemo(() => {
    const strategy = strategyName();
    return strategy ? `${strategy} · ${rangeLabel()}` : rangeLabel();
  });
  const nameValue = () => (form.nameTouched ? form.name : autoName());

  // ---- Validation -----------------------------------------------------------------------------

  const num = (value: string): number | null => {
    if (value.trim() === "") return null;
    const n = Number(value);
    return Number.isFinite(n) ? n : null;
  };

  const errors = createMemo(() => {
    const e: Partial<Record<FieldKey, string>> = {};
    const msg = r().form.errors;
    const name = nameValue().trim();
    if (!name) e.name = msg.required;
    else if ([...name].length > 120) e.name = msg.name_length;
    if (!selectedParams()) e.strategy = form.source === "version" && overview()?.versions.length ? msg.version_gone : msg.strategy;
    const start = parseDay(form.start);
    const end = parseDay(form.end);
    if (!start) e.start = msg.date_invalid;
    if (!end) e.end = msg.date_invalid;
    if (start && end) {
      if (form.start >= form.end) e.dates = msg.date_order;
      else if (form.end > today) e.dates = tpl(msg.date_future, { today });
      else if (daysBetween(form.start, form.end) > MAX_YEARS_DAYS) e.dates = msg.date_span;
    }
    const cash = num(form.cash);
    if (cash === null) e.cash = msg.number;
    else if (cash < 1000 || cash > 1e10) e.cash = msg.cash_range;
    const rf = num(form.rf);
    if (rf === null) e.rf = msg.number;
    else if (rf < 0 || rf > 20) e.rf = tpl(msg.range, { min: "0%", max: "20%" });
    for (const f of COST_FIELDS) {
      const v = num(form.costs[f.key]);
      if (v === null) e[f.key] = msg.number;
      else if (v < f.min || v > f.max) e[f.key] = tpl(msg.range, { min: f.min, max: f.max });
    }
    return e;
  });

  /** Client errors show after the first submit attempt; server errors until the field changes. */
  const fieldError = (key: FieldKey) => serverErrors()[key] ?? (submitted() ? errors()[key] : undefined);
  const touch = (...keys: FieldKey[]) => {
    if (keys.some((k) => serverErrors()[k])) {
      setServerErrors((prev) => {
        const next = { ...prev };
        for (const key of keys) delete next[key];
        return next;
      });
    }
    setGeneralError(null);
  };
  /** Brings the first problem into view (the dialog body scrolls). */
  const revealFirstError = () =>
    requestAnimationFrame(() => {
      const target = formRef?.querySelector<HTMLElement>(".callout.critical, .input.invalid, .error-text");
      target?.scrollIntoView({ block: "center", behavior: "smooth" });
      if (target instanceof HTMLInputElement) target.focus({ preventScroll: true });
    });

  const costModel = (): CostModel => {
    const out = {} as CostModel;
    for (const f of COST_FIELDS) out[f.key] = (num(form.costs[f.key]) ?? 0) / f.factor;
    return out;
  };
  const costsModified = () => {
    const live = settings()?.execution.costs;
    return live ? !costsEqual(costModel(), live) : false;
  };
  const costErrorCount = () => COST_FIELDS.filter((f) => fieldError(f.key)).length;

  const warmupWarning = createMemo(() => {
    const e = earliest();
    if (!e || !parseDay(form.start) || form.start >= e) return null;
    return tpl(r().range.warmup_warning, { earliest: e });
  });

  // ---- Submit ---------------------------------------------------------------------------------

  /** Maps the server's (English) validation messages onto fields. */
  function mapServerError(error: ApiError) {
    const msg = r().form.errors;
    const fields: Partial<Record<FieldKey, string>> = {};
    let general: string | null = null;
    if (error.fields?.length) {
      fields.strategy = tpl(msg.strategy_invalid, { detail: error.fields.map((f) => `${f.path}: ${f.message}`).join("; ") });
    } else {
      const m = error.message ?? "";
      const cost = /^(commission_per_share|commission_min|commission_max_rate|commission_rate|slippage_bps|sell_fee_rate) must be between/.exec(m);
      if (cost) {
        const f = COST_FIELDS.find((x) => x.key === cost[1])!;
        fields[f.key] = tpl(msg.range, { min: f.min, max: f.max });
        setForm("costsOpen", true);
      } else if (/^name must be/.test(m)) fields.name = msg.name_length;
      else if (/^initial cash/.test(m)) fields.cash = msg.cash_range;
      else if (/start date must be before/.test(m)) fields.dates = msg.date_order;
      else if (/limited to 25 years/.test(m)) fields.dates = msg.date_span;
      else if (/^invalid strategy parameters: ?(.*)$/.test(m)) fields.strategy = tpl(msg.strategy_invalid, { detail: m.replace(/^invalid strategy parameters: ?/, "") });
      else if (error.status === 404 && /strategy version/.test(m)) fields.strategy = msg.version_gone;
      else if (/unknown preset/.test(m)) fields.strategy = msg.strategy;
      else general = error.code === "network" ? c().states.network : m || c().states.error;
    }
    setServerErrors(fields);
    setGeneralError(general);
    revealFirstError();
  }

  async function submit(event?: Event) {
    event?.preventDefault();
    if (busy()) return;
    setSubmitted(true);
    if (Object.keys(errors()).length) {
      if (COST_FIELDS.some((f) => errors()[f.key])) setForm("costsOpen", true);
      revealFirstError();
      return;
    }
    setServerErrors({});
    const ov = overview()!;
    const slots: ("open" | "close")[] = form.slots === "both" ? ["open", "close"] : [form.slots];
    const rf = (num(form.rf) ?? 0) / 100;
    const body: Record<string, unknown> = {
      name: nameValue().trim(),
      start: form.start,
      end: form.end,
      initial_cash: num(form.cash),
      costs: costModel(),
      slots,
      frequency: form.frequency,
      benchmark: form.benchmark || null,
      risk_free_rate: rf,
    };
    if (form.source === "active") body.version_id = ov.active!.id;
    else if (form.source === "version") body.version_id = form.versionId;
    else if (form.source === "preset") body.preset_id = form.presetId;
    else body.params = form.customParams;
    setBusy(true);
    try {
      const { backtest } = await api.createBacktest(body);
      toast(r().form.submitted, backtest.name, "success");
      const used = backtest.config.risk_free_rate ?? 0;
      if (Math.abs(used - rf) > 1e-9) {
        toast(tpl(r().form.rf_ignored, { used: fmtPct(used, { dp: 2 }), wanted: fmtPct(rf, { dp: 2 }) }), undefined, "warning", 8000);
      }
      props.onCreated(backtest);
    } catch (error) {
      if (error instanceof ApiError) mapServerError(error);
      else setGeneralError(String(error));
    } finally {
      setBusy(false);
    }
  }

  // ---- Render ---------------------------------------------------------------------------------

  const benchmarks = createMemo(() => {
    const st = settings();
    const symbols = [...(st?.benchmarks.symbols ?? []), UNIVERSE_EW];
    if (form.benchmark && !symbols.includes(form.benchmark)) symbols.unshift(form.benchmark);
    return symbols;
  });

  const unitLabel = (unit: (typeof COST_FIELDS)[number]["unit"]) => {
    const f = r().form;
    return { per_share: f.unit_per_share, usd: f.unit_usd, pct: f.unit_pct, bp: f.unit_bp, per_million: f.unit_per_million }[unit];
  };

  const title = () => (props.prefill.kind === "duplicate" ? r().form.title_duplicate : r().form.title);
  const span = () => (parseDay(form.start) && parseDay(form.end) && form.start < form.end ? yearsBetween(form.start, form.end) : null);

  return (
    <Dialog
      title={title()}
      subtitle={r().form.subtitle}
      onClose={props.onClose}
      wide
      footer={
        <>
          <button class="btn" type="button" onClick={props.onClose}>
            {c().actions.cancel}
          </button>
          <button class="btn primary" type="submit" form="rs-new-backtest" disabled={busy() || !ready()}>
            <Show when={busy()} fallback={<Icon name="play" size={14} />}>
              <span class="rs-btn-spinner" />
            </Show>
            {busy() ? r().form.submitting : r().form.submit}
          </button>
        </>
      }
    >
      <Show
        when={ready()}
        fallback={
          <Show when={loadError()} fallback={<div class="loading-row"><div class="spinner" />{r().form.loading}</div>}>
            <div class="callout critical">
              <Icon name="alert" size={16} />
              <span>{loadError() instanceof ApiError ? (loadError() as ApiError).message : String(loadError())}</span>
            </div>
          </Show>
        }
      >
        <form id="rs-new-backtest" ref={formRef} class="rs-form" onSubmit={submit} novalidate>
          <Show when={generalError()}>
            <div class="callout critical" role="alert">
              <Icon name="alert" size={16} />
              <div>
                <strong>{r().form.error_title}</strong>
                <div>{generalError()}</div>
              </div>
            </div>
          </Show>
          <Show when={notice()}>
            <div class="callout warn">
              <Icon name="info" size={16} />
              <span>{notice()}</span>
            </div>
          </Show>

          <div class="field">
            <label for="bt-name">{r().form.name}</label>
            <input
              id="bt-name"
              class={`input ${fieldError("name") ? "invalid" : ""}`}
              maxLength={120}
              value={nameValue()}
              placeholder={r().form.name_placeholder}
              onInput={(e) => {
                setForm({ name: e.currentTarget.value, nameTouched: true });
                touch("name");
              }}
            />
            <Show when={fieldError("name")}>
              <span class="error-text">{fieldError("name")}</span>
            </Show>
          </div>

          {/* Strategy */}
          <section class="rs-section">
            <h3 class="rs-section-title">{r().form.strategy}</h3>
            <Segmented
              value={form.source}
              options={sourceOptions()}
              onChange={(v) => {
                setForm("source", v);
                touch("strategy");
              }}
              label={r().form.strategy}
            />
            <Show when={form.source === "active"}>
              <Show when={overview()?.active} fallback={<p class="muted small">{r().source.none_active}</p>}>
                {(active) => (
                  <div class="rs-strategy-pick">
                    <div class="row wrap" style={{ gap: "8px" }}>
                      <b>{versionName(active())}</b>
                      <span class="muted xs">#{active().id}</span>
                      <span class="chip green">
                        <span class="dot" />
                        {r().source.active_chip}
                      </span>
                    </div>
                    <span class="muted xs">
                      {actorName(active().created_by)} · {fmtDate(active().created_at)}
                      <Show when={versionBasis()}> · {r().source.preset_prefix}: {versionBasis()}</Show>
                    </span>
                  </div>
                )}
              </Show>
            </Show>
            <Show when={form.source === "version"}>
              <div class="field">
                <select
                  class="select"
                  aria-label={r().source.version}
                  value={form.versionId ?? ""}
                  onChange={(e) => {
                    setForm("versionId", Number(e.currentTarget.value));
                    touch("strategy");
                  }}
                >
                  <For each={overview()?.versions ?? []}>
                    {(v) => (
                      <option value={v.id}>
                        {tpl(r().source.version_option, { name: v.name, id: v.id, date: fmtDate(v.created_at) })}
                        {overview()?.active?.id === v.id ? ` · ${r().source.active_chip}` : ""}
                      </option>
                    )}
                  </For>
                </select>
                <Show when={versionBasis()}>
                  <span class="hint">
                    {r().source.preset_prefix}: {versionBasis()}
                  </span>
                </Show>
              </div>
            </Show>
            <Show when={form.source === "preset"}>
              <div class="field">
                <select
                  class="select"
                  aria-label={r().source.preset}
                  value={form.presetId}
                  onChange={(e) => {
                    setForm("presetId", e.currentTarget.value);
                    touch("strategy");
                  }}
                >
                  <For each={overview()?.presets ?? []}>{(p) => <option value={p.id}>{pickText(p, "name")}</option>}</For>
                </select>
                <span class="hint">{pickText(overview()?.presets.find((p) => p.id === form.presetId), "summary")}</span>
              </div>
            </Show>
            <Show when={form.source === "custom"}>
              <p class="muted small">{r().source.custom_hint}</p>
            </Show>
            <Show when={fieldError("strategy")}>
              <span class="error-text">{fieldError("strategy")}</span>
            </Show>
          </section>

          {/* Period & capital */}
          <section class="rs-section">
            <h3 class="rs-section-title">{r().form.period}</h3>
            <div class="row wrap" style={{ gap: "10px" }}>
              <Segmented
                value={form.range}
                label={r().form.period}
                options={[
                  { value: "1Y" as RangeMode, label: r().range.y1 },
                  { value: "3Y" as RangeMode, label: r().range.y3 },
                  { value: "5Y" as RangeMode, label: r().range.y5 },
                  { value: "MAX" as RangeMode, label: r().range.max },
                  { value: "custom" as RangeMode, label: r().range.custom },
                ]}
                onChange={(v) => {
                  applyRange(v);
                  touch("dates", "start", "end");
                }}
              />
              <Show when={span() != null}>
                <span class="muted xs">{tpl(r().range.span, { years: fmtYears(span()!) })}</span>
              </Show>
            </div>
            <div class="rs-form-grid three">
              <div class="field">
                <label for="bt-start">{r().form.start}</label>
                <input
                  id="bt-start"
                  type="date"
                  class={`input ${fieldError("start") || fieldError("dates") ? "invalid" : ""}`}
                  value={form.start}
                  max={form.end || today}
                  onInput={(e) => {
                    setForm({ start: e.currentTarget.value, range: "custom" });
                    touch("dates", "start");
                  }}
                />
                <Show when={fieldError("start")}>
                  <span class="error-text">{fieldError("start")}</span>
                </Show>
              </div>
              <div class="field">
                <label for="bt-end">{r().form.end}</label>
                <input
                  id="bt-end"
                  type="date"
                  class={`input ${fieldError("end") || fieldError("dates") ? "invalid" : ""}`}
                  value={form.end}
                  max={today}
                  onInput={(e) => {
                    setForm({ end: e.currentTarget.value, range: "custom" });
                    touch("dates", "end");
                  }}
                />
                <Show when={fieldError("end")}>
                  <span class="error-text">{fieldError("end")}</span>
                </Show>
              </div>
              <div class="field">
                <label for="bt-cash">{r().form.capital}</label>
                <div class="input-group">
                  <input
                    id="bt-cash"
                    class={`input num ${fieldError("cash") ? "invalid" : ""}`}
                    type="number"
                    inputmode="decimal"
                    min="1000"
                    step="10000"
                    value={form.cash}
                    onInput={(e) => {
                      setForm("cash", e.currentTarget.value);
                      touch("cash");
                    }}
                  />
                  <span class="addon">USD</span>
                </div>
                <Show when={fieldError("cash")}>
                  <span class="error-text">{fieldError("cash")}</span>
                </Show>
              </div>
            </div>
            <Show when={fieldError("dates")}>
              <span class="error-text">{fieldError("dates")}</span>
            </Show>
            <Show when={coverage() && selectedParams()}>
              <p class="muted xs">
                {tpl(r().range.data_hint, {
                  from: coverage()!.first,
                  to: coverage()!.last,
                  days: maxLookback(selectedParams()!),
                  earliest: earliest() ?? "—",
                })}
              </p>
            </Show>
            <Show when={warmupWarning()}>
              <div class="callout warn rs-compact-callout">
                <Icon name="alert" size={15} />
                <span>{warmupWarning()}</span>
              </div>
            </Show>
          </section>

          {/* Execution */}
          <section class="rs-section">
            <h3 class="rs-section-title">{r().form.execution}</h3>
            <div class="rs-form-grid two">
              <div class="field">
                <span class="field-label">{r().form.slots}</span>
                <Segmented
                  value={form.slots}
                  label={r().form.slots}
                  options={[
                    { value: "open" as SlotChoice, label: r().slots.open, title: r().slots.open_hint },
                    { value: "close" as SlotChoice, label: r().slots.close, title: r().slots.close_hint },
                    { value: "both" as SlotChoice, label: r().slots.both, title: r().slots.both_hint },
                  ]}
                  onChange={(v) => setForm("slots", v)}
                />
                <span class="hint">{form.slots === "open" ? r().slots.open_hint : form.slots === "close" ? r().slots.close_hint : r().slots.both_hint}</span>
              </div>
              <div class="field">
                <span class="field-label">{r().form.frequency}</span>
                <Segmented
                  value={form.frequency}
                  label={r().form.frequency}
                  options={[
                    { value: "daily" as const, label: r().frequency.daily },
                    { value: "weekly" as const, label: r().frequency.weekly },
                    { value: "monthly" as const, label: r().frequency.monthly },
                  ]}
                  onChange={(v) => setForm("frequency", v)}
                />
                <span class="hint">{r().form.frequency_hint}</span>
              </div>
              <div class="field">
                <label for="bt-bench">{r().form.benchmark}</label>
                <select id="bt-bench" class="select" value={form.benchmark} onChange={(e) => setForm("benchmark", e.currentTarget.value)}>
                  <For each={benchmarks()}>{(symbol) => <option value={symbol}>{benchLong(symbol, universe())}</option>}</For>
                </select>
                <span class="hint">{r().form.benchmark_hint}</span>
              </div>
              <div class="field">
                <label for="bt-rf">{r().form.rf}</label>
                <div class="input-group">
                  <input
                    id="bt-rf"
                    class={`input num ${fieldError("rf") ? "invalid" : ""}`}
                    type="number"
                    inputmode="decimal"
                    min="0"
                    max="20"
                    step="0.25"
                    value={form.rf}
                    onInput={(e) => {
                      setForm("rf", e.currentTarget.value);
                      touch("rf");
                    }}
                  />
                  <span class="addon">%</span>
                </div>
                <Show when={fieldError("rf")} fallback={<span class="hint">{r().form.rf_hint}</span>}>
                  <span class="error-text">{fieldError("rf")}</span>
                </Show>
              </div>
            </div>
          </section>

          {/* Costs */}
          <section class="rs-section">
            <button type="button" class="rs-disclosure" aria-expanded={form.costsOpen} onClick={() => setForm("costsOpen", !form.costsOpen)}>
              <Icon name={form.costsOpen ? "chevron_down" : "chevron_right"} size={16} />
              <span class="rs-disclosure-title">{r().form.costs}</span>
              <span class={`chip ${costsModified() ? "orange" : ""}`}>{costsModified() ? r().form.costs_modified : r().form.costs_same}</span>
              <Show when={costErrorCount() > 0}>
                <span class="chip red">
                  <Icon name="alert" size={11} />
                  {costErrorCount()}
                </span>
              </Show>
            </button>
            <Show when={!form.costsOpen}>
              <p class="muted xs rs-costs-summary">{costsSummary(costModel())}</p>
            </Show>
            <Show when={form.costsOpen}>
              <div class="rs-form-grid three">
                <For each={COST_FIELDS}>
                  {(f) => (
                    <div class="field">
                      <label for={`bt-${f.key}`}>{r().form[f.key]}</label>
                      <div class="input-group">
                        <input
                          id={`bt-${f.key}`}
                          class={`input num ${fieldError(f.key) ? "invalid" : ""}`}
                          type="number"
                          inputmode="decimal"
                          min={f.min}
                          max={f.max}
                          step={f.step}
                          value={form.costs[f.key]}
                          onInput={(e) => {
                            setForm("costs", f.key, e.currentTarget.value);
                            touch(f.key);
                          }}
                        />
                        <span class="addon">{unitLabel(f.unit)}</span>
                      </div>
                      <Show when={fieldError(f.key)} fallback={<span class="hint">{r().form[`${f.key}_hint` as const]}</span>}>
                        <span class="error-text">{fieldError(f.key)}</span>
                      </Show>
                    </div>
                  )}
                </For>
              </div>
              <div class="row">
                <button
                  type="button"
                  class="btn sm ghost"
                  disabled={!costsModified()}
                  onClick={() => {
                    const live = settings()?.execution.costs;
                    if (live) setForm("costs", costToDisplay(live));
                    touch(...COST_FIELDS.map((f) => f.key));
                  }}
                >
                  <Icon name="refresh" size={14} />
                  {r().form.costs_reset}
                </button>
              </div>
            </Show>
          </section>

          <p class="rs-form-note">
            <Icon name="info" size={14} />
            <span>{r().form.method_note}</span>
          </p>
        </form>
      </Show>
    </Dialog>
  );
}
