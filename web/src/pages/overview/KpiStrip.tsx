import { Show } from "solid-js";
import { Kpi, Pct } from "@/components/ui";
import { tpl } from "@/i18n";
import { overviewText } from "@/i18n/overview";
import { fmtMoney, fmtPct, moneyPolarity, polarity } from "@/lib/format";
import type { Valuation } from "@/lib/types";

export function KpiStrip(props: { valuation: Valuation; invested_target: number | null; universe: number }) {
  const t = overviewText;
  const v = () => props.valuation;
  const held = () => v().positions.filter((p) => p.qty > 0).length;
  return (
    <div class="kpis overview-kpis">
      <Kpi
        hero
        label={t().kpi.nav}
        value={<span class="num">{fmtMoney(v().nav)}</span>}
        delta={
          <Show when={v().day_pnl != null} fallback={<span class="muted">{t().kpi.day_pnl_none}</span>}>
            <span class={`num ${polarity(v().day_return, 2)}`}>
              {t().kpi.day_pnl} {fmtMoney(v().day_pnl, { sign: true })} ({fmtPct(v().day_return, { sign: true })})
            </span>
          </Show>
        }
      />
      <Kpi
        label={t().kpi.total_return}
        value={<Pct value={v().total_return} />}
        delta={<span class="muted">{tpl(t().kpi.since, { date: v().account.inception_date })}</span>}
      />
      <Kpi
        label={t().kpi.exposure}
        value={<span class="num">{fmtPct(v().exposure, { dp: 1 })}</span>}
        delta={
          <Show when={props.invested_target != null}>
            <span class="muted">{tpl(t().kpi.exposure_target, { pct: fmtPct(props.invested_target, { dp: 1 }) })}</span>
          </Show>
        }
      />
      <Kpi
        label={t().kpi.cash}
        value={<span class="num">{fmtMoney(v().cash, { compact: true })}</span>}
        delta={<span class="muted num">{fmtPct(v().nav > 0 ? v().cash / v().nav : null, { dp: 1 })}</span>}
      />
      <Kpi
        label={t().kpi.unrealized}
        value={<span class={`num ${moneyPolarity(v().unrealized_pnl, 0)}`}>{fmtMoney(v().unrealized_pnl, { sign: true, compact: true })}</span>}
        delta={<span class="muted num">{tpl(t().kpi.realized, { value: fmtMoney(v().realized_pnl + v().dividends, { sign: true, compact: true }) })}</span>}
      />
      <Kpi
        label={t().kpi.drawdown}
        value={<span class={`num ${polarity(v().drawdown, 2)}`}>{fmtPct(v().drawdown, { dp: 2 })}</span>}
        delta={<span class="muted num">{tpl(t().kpi.peak, { value: fmtMoney(v().peak_nav, { compact: true }) })}</span>}
      />
      <Kpi
        label={t().kpi.positions}
        value={<span class="num">{held()}</span>}
        delta={<span class="muted">{tpl(t().kpi.positions_of, { n: props.universe })}</span>}
      />
    </div>
  );
}
