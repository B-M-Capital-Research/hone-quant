import { For, Show, createMemo, createSignal } from "solid-js";
import { tpl } from "@/i18n";
import { common } from "@/i18n/common";
import { settingsText } from "@/i18n/settings";
import { api } from "@/lib/api";
import { fmtMoney } from "@/lib/format";
import type { CostModel, ExecutionSettings } from "@/lib/types";
import { type NumSpec, normalizeNumText, parseNum, rangeMessage, toText } from "./num";
import { type Errors, Field, FormCard, Gate, GroupTitle, NumInput, createSectionForm, useSettings, withDefault } from "./shared";

type CostKey = keyof CostModel;
type GuardKey = "max_quote_age_secs" | "max_price_deviation" | "max_daily_move" | "quote_poll_secs";
type Key = CostKey | GuardKey;
type Form = Record<Key, string>;

const COST_KEYS: CostKey[] = ["commission_per_share", "commission_min", "commission_max_rate", "commission_rate", "slippage_bps", "sell_fee_rate"];
const GUARD_KEYS: GuardKey[] = ["max_quote_age_secs", "max_price_deviation", "max_daily_move", "quote_poll_secs"];
const KEYS: Key[] = [...COST_KEYS, ...GUARD_KEYS];

const u = () => settingsText().units;
/** Ranges mirror `CostModel::validate` and `ExecutionSettings::validate`, in display units. */
const SPECS: Record<Key, NumSpec> = {
  commission_per_share: { min: 0, max: 1, prefix: "$" },
  commission_min: { min: 0, max: 100, prefix: "$" },
  commission_max_rate: { min: 0, max: 5, div: 100, unit: () => u().pct },
  commission_rate: { min: 0, max: 1, div: 100, unit: () => u().pct },
  slippage_bps: { min: 0, max: 200, unit: () => u().bp },
  sell_fee_rate: { min: 0, max: 1000, div: 1e6, prefix: "$" },
  max_quote_age_secs: { min: 30, max: 3600, int: true, unit: () => u().seconds },
  max_price_deviation: { min: 0.2, max: 20, div: 100, unit: () => u().pct },
  max_daily_move: { min: 5, max: 90, div: 100, unit: () => u().pct },
  quote_poll_secs: { min: 15, max: 900, int: true, unit: () => u().seconds },
};

const isCost = (k: Key): k is CostKey => (COST_KEYS as string[]).includes(k);
const storedOf = (v: ExecutionSettings, k: Key): number => (isCost(k) ? v.costs[k] : v[k]);

function toForm(v: ExecutionSettings): Form {
  return Object.fromEntries(KEYS.map((k) => [k, toText(storedOf(v, k), SPECS[k])])) as Form;
}

function parse(f: Form, base: ExecutionSettings): { value: ExecutionSettings | null; errors: Errors } {
  const errors: Errors = {};
  const out: ExecutionSettings = { ...base, costs: { ...base.costs } };
  for (const k of KEYS) {
    const r = parseNum(f[k], SPECS[k]);
    if (!r.ok) errors[k] = r.error;
    else if (isCost(k)) out.costs[k] = r.stored;
    else out[k] = r.stored;
  }
  return { value: Object.keys(errors).length ? null : out, errors };
}

function fieldFor(message: string): string | null {
  return KEYS.find((k) => message.includes(k)) ?? null;
}

export default function ExecutionSection() {
  const { bundle } = useSettings();
  return <Gate loader={bundle}>{(b) => <ExecutionForm source={() => b().execution} defaults={() => b().defaults.execution} />}</Gate>;
}

function ExecutionForm(props: { source: () => ExecutionSettings; defaults: () => ExecutionSettings }) {
  const t = settingsText;
  const { bundle } = useSettings();
  const f = createSectionForm<ExecutionSettings, Form>({
    source: props.source,
    defaults: props.defaults,
    toForm,
    parse,
    submit: (v) => api.putSettings("execution", v),
    after: () => bundle.reload(),
    fieldFor,
    describe: (k) => (k in SPECS ? rangeMessage(SPECS[k as Key]) : undefined),
    savedLabel: () => t().execution.saved_label,
  });

  const units: Record<Key, () => { prefix?: string; unit: string }> = {
    commission_per_share: () => ({ prefix: "$", unit: u().per_share }),
    commission_min: () => ({ prefix: "$", unit: u().per_order }),
    commission_max_rate: () => ({ unit: u().of_value }),
    commission_rate: () => ({ unit: u().of_value }),
    slippage_bps: () => ({ unit: "bp" }),
    sell_fee_rate: () => ({ prefix: "$", unit: u().per_million }),
    max_quote_age_secs: () => ({ unit: u().sec_short }),
    max_price_deviation: () => ({ unit: "%" }),
    max_daily_move: () => ({ unit: "%" }),
    quote_poll_secs: () => ({ unit: u().sec_short }),
  };

  const labels = (): Record<Key, [string, string]> => {
    const e = t().execution;
    return {
      commission_per_share: [e.per_share, e.per_share_hint],
      commission_min: [e.min, e.min_hint],
      commission_max_rate: [e.max_rate, e.max_rate_hint],
      commission_rate: [e.rate, e.rate_hint],
      slippage_bps: [e.slippage, e.slippage_hint],
      sell_fee_rate: [e.sec_fee, e.sec_fee_hint],
      max_quote_age_secs: [e.quote_age, e.quote_age_hint],
      max_price_deviation: [e.deviation, e.deviation_hint],
      max_daily_move: [e.daily_move, e.daily_move_hint],
      quote_poll_secs: [e.poll, e.poll_hint],
    };
  };

  /** "Default $0.005/share", "Default 3%", … */
  const defaultText = (k: Key) => {
    const unit = units[k]();
    const text = toText(storedOf(props.defaults(), k), SPECS[k]);
    const suffix = unit.unit === "%" || unit.unit === u().of_value ? "%" : unit.unit === "bp" ? " bp" : unit.unit.startsWith("/") ? unit.unit : ` ${unit.unit}`;
    return `${unit.prefix ?? ""}${text}${suffix}`;
  };

  const field = (k: Key) => (
    <Field
      id={`exec-${k}`}
      label={labels()[k][0]}
      hint={withDefault(labels()[k][1], defaultText(k))}
      error={f.error(k)}
      changed={f.changed(k)}
    >
      <NumInput
        id={`exec-${k}`}
        value={f.form[k]}
        onInput={(v) => f.set(k, v)}
        onBlur={() => f.touch(k)}
        prefix={units[k]().prefix}
        unit={units[k]().unit}
        int={SPECS[k].int}
        invalid={!!f.error(k)}
        disabled={!f.canEdit()}
      />
    </Field>
  );

  /** Valid form values over the saved ones, for the live example. */
  const model = createMemo<CostModel | null>(() => {
    const out = { ...f.baseline().costs };
    for (const k of COST_KEYS) {
      const r = parseNum(f.form[k], SPECS[k]);
      if (!r.ok) return null;
      out[k] = r.stored;
    }
    return out;
  });

  return (
    <>
      <FormCard f={f} title={t().execution.form_title} sub={t().execution.form_sub}>
        <div class="stack" style={{ gap: "26px" }}>
          <section>
            <GroupTitle sub={t().execution.costs_sub}>{t().execution.costs_title}</GroupTitle>
            <div class="form-grid">
              <For each={COST_KEYS}>{(k) => field(k)}</For>
            </div>
          </section>
          <section>
            <GroupTitle sub={t().execution.guards_sub}>{t().execution.guards_title}</GroupTitle>
            <div class="form-grid">
              <For each={GUARD_KEYS}>{(k) => field(k)}</For>
            </div>
          </section>
        </div>
      </FormCard>
      <CostExample model={model()} />
    </>
  );
}

// ---------------------------------------------------------------------------------------------
// Worked example — the same arithmetic as `quant_core::costs::CostModel`.
// ---------------------------------------------------------------------------------------------

interface Breakdown {
  side: "buy" | "sell";
  price: number;
  notional: number;
  raw: number;
  minApplied: boolean;
  capped: boolean;
  cap: number;
  extra: number;
  commission: number;
  fees: number;
  slippage: number;
  total: number;
  cash: number;
  bps: number;
}

function breakdown(m: CostModel, side: "buy" | "sell", qty: number, ref: number): Breakdown {
  const sign = side === "buy" ? 1 : -1;
  const price = ref * (1 + (sign * m.slippage_bps) / 10_000);
  const notional = qty * price;
  const raw = qty * m.commission_per_share;
  let perShare = Math.max(raw, m.commission_min);
  const cap = notional * m.commission_max_rate;
  let capped = false;
  if (m.commission_max_rate > 0 && perShare > cap) {
    perShare = cap;
    capped = true;
  }
  const extra = notional * m.commission_rate;
  const commission = perShare + extra;
  const fees = side === "sell" ? notional * m.sell_fee_rate : 0;
  const slippage = qty * Math.abs(price - ref);
  const total = commission + fees + slippage;
  const cash = side === "buy" ? -(notional + commission + fees) : notional - commission - fees;
  return {
    side,
    price,
    notional,
    raw,
    minApplied: !capped && raw < m.commission_min,
    capped,
    cap,
    extra,
    commission,
    fees,
    slippage,
    total,
    cash,
    bps: qty * ref > 0 ? (total / (qty * ref)) * 10_000 : 0,
  };
}

const px = (n: number) => `$${n.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 4 })}`;
const money = (n: number) => fmtMoney(n);

function CostExample(props: { model: CostModel | null }) {
  const t = settingsText;
  const [qtyText, setQtyText] = createSignal("100");
  const [priceText, setPriceText] = createSignal("150");
  const qty = () => {
    const n = Number(normalizeNumText(qtyText()));
    return Number.isFinite(n) && n > 0 && n <= 1e9 ? n : null;
  };
  const price = () => {
    const n = Number(normalizeNumText(priceText()));
    return Number.isFinite(n) && n > 0 && n <= 1e7 ? n : null;
  };
  const rows = createMemo(() => {
    const m = props.model;
    const q = qty();
    const p = price();
    if (!m || q === null || p === null) return null;
    return { buy: breakdown(m, "buy", q, p), sell: breakdown(m, "sell", q, p), m, q, p };
  });

  const why = (b: Breakdown, m: CostModel, q: number) => {
    const e = t().execution;
    const base = { qty: q.toLocaleString("en-US"), rate: px(m.commission_per_share), raw: money(b.raw) };
    let text = b.capped
      ? tpl(e.why_cap, { ...base, cap: money(b.cap), pct: `${+(m.commission_max_rate * 100).toPrecision(6)}%` })
      : b.minApplied
        ? tpl(e.why_min, { ...base, min: money(m.commission_min) })
        : tpl(e.why_plain, base);
    if (b.extra > 0) text += `; ${tpl(e.why_rate, { pct: `${+(m.commission_rate * 100).toPrecision(6)}%`, extra: money(b.extra) })}`;
    return text;
  };

  return (
    <div class="card">
      <div class="card-head">
        <div class="head-text">
          <h2>{t().execution.example_title}</h2>
          <div class="sub">{t().execution.example_sub}</div>
        </div>
      </div>
      <div class="card-body stack" style={{ gap: "16px" }}>
        <div class="example-inputs">
          <div class="field">
            <label for="ex-qty">{t().execution.example_qty}</label>
            <div class="input-group num-group">
              <input id="ex-qty" class="input num" inputmode="numeric" value={qtyText()} onInput={(e) => setQtyText(e.currentTarget.value)} />
              <span class="addon">{common().units.shares}</span>
            </div>
          </div>
          <div class="field">
            <label for="ex-price">{t().execution.example_price}</label>
            <div class="input-group num-group has-prefix">
              <span class="addon pre">$</span>
              <input id="ex-price" class="input num" inputmode="decimal" value={priceText()} onInput={(e) => setPriceText(e.currentTarget.value)} />
              <span class="addon">USD</span>
            </div>
          </div>
        </div>
        <Show when={rows()} fallback={<p class="muted small">{t().execution.invalid_example}</p>}>
          {(r) => (
            <>
              <div class="table-wrap">
                <table class="table compact example-table">
                  <thead>
                    <tr>
                      <th />
                      <th class="r">{t().execution.example_buy}</th>
                      <th class="r">{t().execution.example_sell}</th>
                    </tr>
                  </thead>
                  <tbody>
                    <tr>
                      <td>{t().execution.row_exec_price}</td>
                      <td class="r">{px(r().buy.price)}</td>
                      <td class="r">{px(r().sell.price)}</td>
                    </tr>
                    <tr>
                      <td>{t().execution.row_notional}</td>
                      <td class="r">{money(r().buy.notional)}</td>
                      <td class="r">{money(r().sell.notional)}</td>
                    </tr>
                    <tr>
                      <td>
                        {t().execution.row_commission}
                        <div class="muted xs">{why(r().buy, r().m, r().q)}</div>
                      </td>
                      <td class="r">{money(r().buy.commission)}</td>
                      <td class="r">{money(r().sell.commission)}</td>
                    </tr>
                    <tr>
                      <td>{t().execution.row_fees}</td>
                      <td class="r muted">—</td>
                      <td class="r">{money(r().sell.fees)}</td>
                    </tr>
                    <tr>
                      <td>{t().execution.row_slippage}</td>
                      <td class="r">{money(r().buy.slippage)}</td>
                      <td class="r">{money(r().sell.slippage)}</td>
                    </tr>
                  </tbody>
                  <tfoot>
                    <tr>
                      <td>{t().execution.row_total}</td>
                      <td class="r">
                        {money(r().buy.total)} <span class="muted xs">({r().buy.bps.toFixed(1)} bp)</span>
                      </td>
                      <td class="r">
                        {money(r().sell.total)} <span class="muted xs">({r().sell.bps.toFixed(1)} bp)</span>
                      </td>
                    </tr>
                    <tr>
                      <td>{t().execution.row_cash}</td>
                      <td class="r">{fmtMoney(r().buy.cash, { sign: true })}</td>
                      <td class="r">{fmtMoney(r().sell.cash, { sign: true })}</td>
                    </tr>
                  </tfoot>
                </table>
              </div>
              <p class="small subtle example-summary">
                {tpl(t().execution.summary, {
                  qty: r().q.toLocaleString("en-US"),
                  price: px(r().p),
                  commission: money(r().buy.commission),
                  slippage: money(r().buy.slippage),
                  fees: money(r().sell.fees),
                  total: money(r().buy.total),
                  bps: r().buy.bps.toFixed(1),
                })}
              </p>
            </>
          )}
        </Show>
      </div>
    </div>
  );
}
