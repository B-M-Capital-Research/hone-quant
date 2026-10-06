import { For, Show, createMemo, createSignal } from "solid-js";
import { tpl } from "@/i18n";
import { settingsText } from "@/i18n/settings";
import { api } from "@/lib/api";
import { fmtPct } from "@/lib/format";
import type { BenchmarkSettings } from "@/lib/types";
import { type NumSpec, parseNum, rangeMessage, toText } from "./num";
import { type Errors, Field, FormCard, Gate, NumInput, SIcon, createLoader, createSectionForm, localName, useSettings, withDefault } from "./shared";

interface Form {
  symbols: string[];
  primary: string;
  risk_free_rate: string;
}

const MAX = 6;
const SYMBOL = /^\^?[A-Z0-9][A-Z0-9.-]{0,14}$/;
/** Mirrors `BenchmarkSettings::validate`. */
const RFR: NumSpec = { min: 0, max: 20, div: 100, unit: () => settingsText().units.pct };

function parse(f: Form, base: BenchmarkSettings): { value: BenchmarkSettings | null; errors: Errors } {
  const t = settingsText().benchmarks;
  const errors: Errors = {};
  const symbols = f.symbols.map((s) => s.trim().toUpperCase());
  if (symbols.length < 1 || symbols.length > MAX) errors.symbols = t.err_count;
  else if (symbols.some((s) => !SYMBOL.test(s))) errors.symbols = t.err_symbol;
  if (!symbols.includes(f.primary)) errors.primary = t.err_primary;
  const r = parseNum(f.risk_free_rate, RFR);
  if (!r.ok) errors.risk_free_rate = r.error;
  if (Object.keys(errors).length || !r.ok) return { value: null, errors };
  return { value: { ...base, symbols, primary: f.primary, risk_free_rate: r.stored }, errors };
}

function fieldFor(message: string): string | null {
  if (/risk_free_rate/.test(message)) return "risk_free_rate";
  if (/primary/.test(message)) return "primary";
  if (/benchmarks/.test(message)) return "symbols";
  return null;
}

export default function BenchmarksSection() {
  const { bundle } = useSettings();
  return <Gate loader={bundle}>{(b) => <BenchmarksForm source={() => b().benchmarks} defaults={() => b().defaults.benchmarks} />}</Gate>;
}

function BenchmarksForm(props: { source: () => BenchmarkSettings; defaults: () => BenchmarkSettings }) {
  const t = settingsText;
  const { bundle } = useSettings();
  const universe = createLoader(() => api.universe());
  const f = createSectionForm<BenchmarkSettings, Form>({
    source: props.source,
    defaults: props.defaults,
    toForm: (v) => ({ symbols: [...v.symbols], primary: v.primary, risk_free_rate: toText(v.risk_free_rate, RFR) }),
    parse,
    submit: (v) => api.putSettings("benchmarks", v),
    after: () => bundle.reload(),
    fieldFor,
    describe: (k) =>
      k === "risk_free_rate" ? rangeMessage(RFR) : k === "primary" ? t().benchmarks.err_primary : k === "symbols" ? t().benchmarks.err_count : undefined,
    savedLabel: () => t().benchmarks.saved_label,
  });

  const builtins = createMemo(() => universe.value()?.benchmarks ?? []);
  const names = createMemo(() => {
    const map = new Map<string, string>();
    for (const a of universe.value()?.assets ?? []) map.set(a.symbol, localName(a));
    for (const b of builtins()) map.set(b.symbol, localName(b));
    return map;
  });
  const isBuiltin = (s: string) => builtins().some((b) => b.symbol === s);
  const available = () => builtins().filter((b) => !f.form.symbols.includes(b.symbol));

  const [custom, setCustom] = createSignal("");
  const [customError, setCustomError] = createSignal<string | null>(null);

  const setSymbols = (symbols: string[]) => {
    f.set("symbols", symbols);
    f.touch("symbols");
    if (!symbols.includes(f.form.primary) && symbols.length) f.set("primary", symbols[0]);
  };
  const add = (symbol: string) => {
    const s = symbol.trim().toUpperCase();
    const b = t().benchmarks;
    if (!SYMBOL.test(s)) return setCustomError(b.err_symbol);
    if (f.form.symbols.includes(s)) return setCustomError(b.err_duplicate);
    if (f.form.symbols.length >= MAX) return setCustomError(b.err_full);
    setCustomError(null);
    setSymbols([...f.form.symbols, s]);
    setCustom("");
  };
  const remove = (symbol: string) => setSymbols(f.form.symbols.filter((s) => s !== symbol));
  const move = (index: number, delta: number) => {
    const list = [...f.form.symbols];
    const to = index + delta;
    if (to < 0 || to >= list.length) return;
    [list[index], list[to]] = [list[to], list[index]];
    f.set("symbols", list);
  };
  const hasCustom = () => f.form.symbols.some((s) => !isBuiltin(s));

  return (
    <FormCard f={f} title={t().benchmarks.form_title} sub={t().benchmarks.form_sub}>
      <div class="stack" style={{ gap: "24px" }}>
        <Field
          label={t().benchmarks.symbols}
          hint={withDefault(t().benchmarks.symbols_hint, props.defaults().symbols.join(", "))}
          error={f.error("symbols") ?? f.error("primary")}
          changed={f.changed("symbols") || f.changed("primary")}
        >
          <ol class="bench-list" aria-invalid={f.error("symbols") ? "true" : "false"} tabindex={f.error("symbols") ? -1 : undefined}>
            <For each={f.form.symbols}>
              {(symbol, i) => (
                <li class="bench-row" classList={{ primary: f.form.primary === symbol }}>
                  <label class="bench-radio" title={t().benchmarks.primary_hint}>
                    <input
                      type="radio"
                      name="bench-primary"
                      checked={f.form.primary === symbol}
                      disabled={!f.canEdit()}
                      onChange={() => f.set("primary", symbol)}
                      aria-label={`${t().benchmarks.primary}: ${symbol}`}
                    />
                  </label>
                  <span class="ticker">{symbol}</span>
                  <span class="bench-name muted small truncate">{names().get(symbol) ?? ""}</span>
                  <Show when={f.form.primary === symbol}>
                    <span class="chip blue">{t().benchmarks.primary_tag}</span>
                  </Show>
                  <span class="spacer" />
                  <Show when={f.canEdit()}>
                    <button type="button" class="btn ghost icon sm" disabled={i() === 0} onClick={() => move(i(), -1)} aria-label="↑">
                      <SIcon name="arrow_up" size={14} />
                    </button>
                    <button type="button" class="btn ghost icon sm" disabled={i() === f.form.symbols.length - 1} onClick={() => move(i(), 1)} aria-label="↓">
                      <SIcon name="arrow_down" size={14} />
                    </button>
                    <button
                      type="button"
                      class="btn ghost icon sm"
                      onClick={() => remove(symbol)}
                      aria-label={tpl(t().benchmarks.remove, { symbol })}
                      title={tpl(t().benchmarks.remove, { symbol })}
                    >
                      <SIcon name="x" size={14} />
                    </button>
                  </Show>
                </li>
              )}
            </For>
          </ol>
        </Field>

        <Show when={f.canEdit()}>
          <div class="bench-add">
            <Show when={available().length > 0}>
              <div class="field">
                <span class="field-label">{t().benchmarks.builtin}</span>
                <div class="pill-list">
                  <For each={available()}>
                    {(b) => (
                      <button type="button" class="btn sm" disabled={f.form.symbols.length >= MAX} onClick={() => add(b.symbol)}>
                        <SIcon name="plus" size={13} />
                        <span class="ticker">{b.symbol}</span>
                        <span class="muted">{localName(b)}</span>
                      </button>
                    )}
                  </For>
                </div>
              </div>
            </Show>
            <div class="field">
              <label class="field-label" for="bench-custom">
                {t().benchmarks.add_custom}
              </label>
              <div class="row" style={{ gap: "8px", "max-width": "360px" }}>
                <input
                  id="bench-custom"
                  class="input mono"
                  classList={{ invalid: !!customError() }}
                  placeholder={t().benchmarks.custom_placeholder}
                  autocomplete="off"
                  spellcheck={false}
                  maxLength={16}
                  value={custom()}
                  onInput={(e) => {
                    setCustom(e.currentTarget.value.toUpperCase());
                    setCustomError(null);
                  }}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      add(custom());
                    }
                  }}
                />
                <button type="button" class="btn" disabled={!custom().trim()} onClick={() => add(custom())}>
                  {t().benchmarks.add}
                </button>
              </div>
              <Show when={customError()}>
                <div class="error-text">{customError()}</div>
              </Show>
            </div>
            <Show when={hasCustom()}>
              <div class="callout info">
                <SIcon name="info" size={16} />
                <span>{t().benchmarks.custom_note}</span>
              </div>
            </Show>
          </div>
        </Show>

        <div class="form-grid">
          <Field
            id="bench-rfr"
            label={t().benchmarks.rfr}
            hint={withDefault(t().benchmarks.rfr_hint, fmtPct(props.defaults().risk_free_rate, { dp: 1 }))}
            error={f.error("risk_free_rate")}
            changed={f.changed("risk_free_rate")}
          >
            <NumInput
              id="bench-rfr"
              value={f.form.risk_free_rate}
              onInput={(v) => f.set("risk_free_rate", v)}
              onBlur={() => f.touch("risk_free_rate")}
              unit={t().units.per_year}
              invalid={!!f.error("risk_free_rate")}
              disabled={!f.canEdit()}
            />
          </Field>
        </div>
      </div>
    </FormCard>
  );
}
