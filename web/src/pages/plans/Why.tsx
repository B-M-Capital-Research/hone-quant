import { A } from "@solidjs/router";
import { For, Show, createMemo } from "solid-js";
import { SideChip } from "@/components/ui";
import { locale, tpl } from "@/i18n";
import { explainText } from "@/i18n/explain";
import { driverChip, driverText, driverTone, oneLine, orderNotes, orderWhy } from "@/lib/explain";
import { fmtPct } from "@/lib/format";
import type { Headline, OrderExplanation, PlanDetail, PlanExplanation } from "@/lib/types";
import { sectorLabel } from "../overview/util";

const sectorName = (id: string) => sectorLabel(undefined, id);

/** "Why this plan": the headline, the driver chips and the largest orders with their reasons. */
export function WhyCard(props: { detail: PlanDetail; name: (symbol: string) => string }) {
  const t = explainText;
  const x = () => props.detail.explanation;
  const params = () => props.detail.diagnostics.params;
  const largest = createMemo(() =>
    [...(x()?.orders ?? [])].sort((a, b) => Math.abs(b.weight_target - b.weight_before) - Math.abs(a.weight_target - a.weight_before)).slice(0, 5),
  );
  const crossedNames = (symbols: string[]) => symbols.join(locale() === "zh" ? "、" : ", ");
  return (
    <Show when={x()}>
      {(x) => (
        <div class="card why-card">
          <div class="card-head">
            <h3>{t().card.title}</h3>
            <span class="spacer" />
            <span class="muted xs">
              <Show when={x().previous_plan_id} fallback={t().card.first}>
                {(prev) => <A href={`/plans/${prev()}`}>{tpl(t().card.compared, { id: prev() })}</A>}
              </Show>
            </span>
          </div>
          <div class="card-body">
            <p class="why-headline">{oneLine(x().drivers, sectorName)}</p>
            <div class="pill-list" aria-label={t().card.drivers}>
              <For each={x().drivers}>
                {(d) => (
                  <span class={`chip ${driverTone(d)}`} title={driverText(d, sectorName)}>
                    {driverChip(d, sectorName)}
                  </span>
                )}
              </For>
            </div>
            <BreadthLine x={x()} line={params()?.exposure.breadth_sma ?? 200} min={params()?.exposure.min_exposure} max={params()?.exposure.max_exposure} crossed={crossedNames} />
            <Show when={x().no_action?.closest}>
              {(c) => (
                <p class="muted xs">
                  {tpl(t().card.no_action_closest, { symbol: c().symbol, drift: fmtPct(c().drift, { dp: 2, sign: true }), band: fmtPct(c().band, { dp: 2 }) })}
                </p>
              )}
            </Show>
            <Show when={largest().length}>
              <div>
                <div class="kicker">{t().card.largest}</div>
                <ul class="why-orders">
                  <For each={largest()}>
                    {(o) => (
                      <li>
                        <SideChip side={o.side} />
                        <A class="ticker" href={`/universe?symbol=${encodeURIComponent(o.symbol)}`}>
                          {o.symbol}
                        </A>
                        <span class="muted xs truncate why-name">{props.name(o.symbol)}</span>
                        <span class="num xs nowrap">
                          {fmtPct(o.weight_before, { dp: 2 })} → <b>{fmtPct(o.weight_target, { dp: 2 })}</b>
                        </span>
                        <span class="xs why-reason" title={orderNotes(o).join(" · ")}>
                          {orderWhy(o, x(), sectorName)}
                        </span>
                      </li>
                    )}
                  </For>
                </ul>
                <Show when={x().orders.length > largest().length}>
                  <p class="muted xs">{tpl(t().card.all_orders, { n: x().orders.length })}</p>
                </Show>
              </div>
            </Show>
            <Show when={x().cash.derived || x().breadth.source === "reconstructed"}>
              <p class="muted xs">
                {[x().breadth.source === "reconstructed" ? t().card.reconstructed : "", x().cash.derived ? t().card.gap_derived : ""].filter(Boolean).join(" ")}
              </p>
            </Show>
          </div>
        </div>
      )}
    </Show>
  );
}

function BreadthLine(props: { x: PlanExplanation; line: number; min?: number; max?: number; crossed: (s: string[]) => string }) {
  const t = explainText;
  const b = () => props.x.breadth;
  return (
    <div class="muted xs breadth-notes">
      <Show when={b().above_now != null && b().counted_now != null}>
        <span>
          {tpl(t().card.breadth_counts, { above: b().above_now, counted: b().counted_now })}
          {" · "}
          {tpl(t().card.gap, {
            now: fmtPct(props.x.cash.invested_now, { dp: 1 }),
            target: fmtPct(props.x.cash.invested_target, { dp: 1 }),
            gap: (Math.abs(props.x.cash.gap) * 100).toFixed(1),
            band: fmtPct(props.x.cash.band, { dp: 0 }),
          })}
        </span>
      </Show>
      <Show when={b().crossed_below.length}>
        <span>{tpl(t().card.crossed_below, { names: props.crossed(b().crossed_below) })}</span>
      </Show>
      <Show when={b().crossed_above.length}>
        <span>{tpl(t().card.crossed_above, { names: props.crossed(b().crossed_above) })}</span>
      </Show>
      <Show when={props.min != null && props.max != null}>
        <span>{tpl(t().card.breadth_line, { line: props.line, min: props.min!.toFixed(2), span: (props.max! - props.min!).toFixed(2) })}</span>
      </Show>
    </div>
  );
}

/** The orders table's "Why" cell. */
export function OrderWhyCell(props: { order: OrderExplanation | undefined; explanation: PlanExplanation | null }) {
  return (
    <Show when={props.order} fallback={<span class="muted">—</span>}>
      {(o) => (
        <span class="xs why-reason" title={orderNotes(o()).join(" · ")}>
          {orderWhy(o(), props.explanation, sectorName)}
          <Show when={o().band_waived}>
            <span class="chip outline why-flag">{explainText().order_why.waived}</span>
          </Show>
        </span>
      )}
    </Show>
  );
}

/** One line of "why" for lists (Plans, Overview). */
export function WhyLine(props: { headline: Headline | null | undefined; n?: number; wrap?: boolean }) {
  const text = () => (props.headline ? oneLine(props.headline.drivers, sectorName, props.n ?? 2) : "");
  return (
    <Show when={text()}>
      <div class={`why-line muted xs ${props.wrap ? "wrap" : ""}`} title={props.headline ? oneLine(props.headline.drivers, sectorName, 6) : undefined}>
        {text()}
      </div>
    </Show>
  );
}
