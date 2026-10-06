/**
 * Cash ledger tab of /trades. GET /api/ledger only takes a limit (no filters, offset or total),
 * so the latest entries are loaded once and filtered, summarised and paged in the browser.
 */
import { For, Match, Show, Switch, createEffect, createMemo, createSignal, on } from "solid-js";
import { Empty, ErrorState, Kpi, Loading, Money } from "@/components/ui";
import { tpl } from "@/i18n";
import { tradesText } from "@/i18n/trades";
import { api } from "@/lib/api";
import { DASH, MARKET_TZ, fmtDateTime, fmtDual, fmtMoney, fmtNum, fmtPrice, fmtQty } from "@/lib/format";
import type { LedgerEntry } from "@/lib/types";
import { DualTime, Pager, SymbolCell } from "./components";
import { useQuery } from "./query";
import { ExportButton, Summary, type TabProps, Toolbar, csvName, exportRows } from "./trades-common";
import { dateIn, num } from "./util";

const LIMIT = 5000;
const KINDS: LedgerEntry["kind"][] = ["trade", "dividend", "deposit", "adjustment"];
const KIND_TONE: Record<string, string> = { deposit: "blue", trade: "", dividend: "green", adjustment: "yellow" };

/** Localises the notes the server writes ("buy 55 @ 62.65131", "initial paper capital", "12 × $0.25"). */
export function ledgerNote(entry: LedgerEntry): string {
  const n = tradesText().ledger.notes;
  const note = entry.note.trim();
  if (entry.kind === "deposit" && note === "initial paper capital") return n.initial;
  const trade = /^(buy|sell)\s+([\d.]+)\s+@\s+([\d.]+)$/.exec(note);
  if (trade) return tpl(trade[1] === "buy" ? n.buy : n.sell, { qty: fmtQty(trade[2]), price: `$${fmtPrice(trade[3])}` });
  const dividend = /^([\d.]+)\s+×\s+\$([\d.]+)$/.exec(note);
  if (dividend) return tpl(n.dividend, { qty: fmtQty(dividend[1]), amount: `$${dividend[2]}` });
  return note;
}

export function LedgerTab(props: TabProps) {
  const t = tradesText;
  const f = () => props.filters;
  const [offset, setOffset] = createSignal(0);
  const [limit, setLimit] = createSignal(50);
  createEffect(on(() => [f().symbol, f().from, f().to, f().kind], () => setOffset(0), { defer: true }));

  const ledger = useQuery(
    () => ({ tick: props.tick() }),
    () => api.ledger(LIMIT),
  );
  const entries = () => ledger.data()?.entries ?? [];

  /** Entries matching symbol and dates (kind applied separately so the kind menu can show counts). */
  const base = createMemo(() => {
    const { symbol, from, to } = f();
    return entries().filter((e) => {
      if (symbol && e.symbol !== symbol) return false;
      if (from || to) {
        const day = dateIn(new Date(e.ts).getTime(), MARKET_TZ);
        if (from && day < from) return false;
        if (to && day > to) return false;
      }
      return true;
    });
  });
  const counts = createMemo(() => {
    const out: Record<string, number> = {};
    for (const e of base()) out[e.kind] = (out[e.kind] ?? 0) + 1;
    return out;
  });
  const filtered = createMemo(() => (f().kind ? base().filter((e) => e.kind === f().kind) : base()));
  const visible = createMemo(() => filtered().slice(offset(), offset() + limit()));

  const summary = createMemo(() => {
    const s = { count: 0, inflow: 0, outflow: 0, dividends: 0, balance: null as number | null, at: null as string | null };
    for (const e of filtered()) {
      s.count += 1;
      const amount = num(e.amount);
      if (amount >= 0) s.inflow += amount;
      else s.outflow += amount;
      if (e.kind === "dividend") s.dividends += amount;
    }
    const newest = filtered()[0];
    if (newest) {
      s.balance = num(newest.balance_after);
      s.at = newest.ts;
    }
    return s;
  });

  const exportCsv = () => {
    const tc = t().csv;
    const cols = t().ledger.cols;
    const header = [tc.id, tc.time_utc, tc.local_time, cols.kind, cols.symbol, cols.amount, cols.balance, cols.note, tc.ref_type, tc.ref_id];
    const rows = filtered().map((e) => [
      e.id,
      e.ts,
      fmtDateTime(e.ts),
      t().ledger.kinds[e.kind] ?? e.kind,
      e.symbol ?? "",
      e.amount,
      e.balance_after,
      ledgerNote(e),
      e.ref_type ?? "",
      e.ref_id ?? "",
    ]);
    exportRows(csvName("cash-ledger", f()), header, rows, null);
  };

  const ready = () => !!ledger.data();
  const limited = () => entries().length >= LIMIT;

  return (
    <>
      <Toolbar
        filters={f()}
        setFilters={props.setFilters}
        names={props.names()}
        dateHint={t().filter.date_hint_ledger}
        extraActive={!!f().kind}
        extra={
          <div class="act-field">
            <label class="act-field-label" for="ledger-kind">
              {t().filter.kind}
            </label>
            <select
              id="ledger-kind"
              class="select act-select"
              classList={{ set: !!f().kind }}
              value={f().kind}
              onChange={(e) => props.setFilters({ kind: e.currentTarget.value })}
            >
              <option value="">
                {t().filter.all_kinds}
                {ready() ? ` (${fmtNum(base().length, 0)})` : ""}
              </option>
              <For each={KINDS.filter((k) => counts()[k] || k === f().kind)}>
                {(k) => (
                  <option value={k}>
                    {t().ledger.kinds[k]} ({fmtNum(counts()[k] ?? 0, 0)})
                  </option>
                )}
              </For>
            </select>
          </div>
        }
      />

      <Summary loading={ledger.loading()} note={limited() ? tpl(t().ledger.limited, { n: fmtNum(LIMIT, 0) }) : null}>
        <Kpi label={t().ledger.kpi.count} value={<span class="num">{ready() ? fmtNum(summary().count, 0) : DASH}</span>} />
        <Kpi label={t().ledger.kpi.inflow} value={ready() ? <Money value={summary().inflow} /> : <span class="num">{DASH}</span>} />
        <Kpi label={t().ledger.kpi.outflow} value={ready() ? <Money value={summary().outflow} /> : <span class="num">{DASH}</span>} />
        <Kpi
          label={t().ledger.kpi.net}
          value={ready() ? <Money value={summary().inflow + summary().outflow} signed /> : <span class="num">{DASH}</span>}
        />
        <Kpi label={t().ledger.kpi.dividends} value={ready() ? <Money value={summary().dividends} /> : <span class="num">{DASH}</span>} />
        <Kpi
          label={t().ledger.kpi.balance}
          value={<span class="num">{summary().balance === null ? DASH : fmtMoney(summary().balance)}</span>}
          delta={
            <Show when={summary().at}>
              <span class="muted" title={fmtDual(summary().at, true)}>
                {tpl(t().ledger.kpi.balance_sub, { time: fmtDual(summary().at, true).split(" · ")[0] })}
              </span>
            </Show>
          }
        />
      </Summary>

      <section class="card">
        <div class="card-head">
          <div>
            <h2>{t().ledger.card}</h2>
            <div class="sub">{t().ledger.sub}</div>
          </div>
          <span class="spacer" />
          <ExportButton onClick={exportCsv} disabled={!ready() || !filtered().length} />
        </div>
        <div class="card-body flush">
          <Switch>
            <Match when={ledger.error() && !ledger.data()}>
              <ErrorState error={ledger.error()} onRetry={ledger.refetch} />
            </Match>
            <Match when={!ledger.data()}>
              <Loading />
            </Match>
            <Match when={filtered().length === 0}>
              <Empty title={t().ledger.empty} icon="trades">
                <span>{t().ledger.empty_hint}</span>
              </Empty>
            </Match>
            <Match when={true}>
              <div class="table-wrap" classList={{ refetching: ledger.loading() }}>
                <table class="table compact act-table">
                  <thead>
                    <tr>
                      <th>{t().ledger.cols.time}</th>
                      <th>{t().ledger.cols.kind}</th>
                      <th>{t().ledger.cols.symbol}</th>
                      <th class="r">{t().ledger.cols.amount}</th>
                      <th class="r">{t().ledger.cols.balance}</th>
                      <th>{t().ledger.cols.note}</th>
                    </tr>
                  </thead>
                  <tbody>
                    <For each={visible()}>
                      {(e) => (
                        <tr>
                          <td>
                            <DualTime value={e.ts} />
                          </td>
                          <td>
                            <span class={`chip ${KIND_TONE[e.kind] ?? ""}`}>{t().ledger.kinds[e.kind] ?? e.kind}</span>
                          </td>
                          <td>
                            <SymbolCell symbol={e.symbol} names={props.names()} onFilter={(symbol) => props.setFilters({ symbol })} />
                          </td>
                          <td class="r">
                            <Money value={e.amount} signed />
                          </td>
                          <td class="r">{fmtMoney(e.balance_after)}</td>
                          <td class="act-note">{ledgerNote(e)}</td>
                        </tr>
                      )}
                    </For>
                  </tbody>
                </table>
              </div>
            </Match>
          </Switch>
        </div>
        <Show when={ready() && filtered().length > 0}>
          <Pager
            offset={offset()}
            limit={limit()}
            total={filtered().length}
            onOffset={setOffset}
            onLimit={(n) => {
              setLimit(n);
              setOffset(0);
            }}
          />
        </Show>
      </section>
    </>
  );
}
