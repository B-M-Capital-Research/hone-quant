import { A } from "@solidjs/router";
import { For, Show, createMemo } from "solid-js";
import { Chart } from "@/components/Chart";
import { locale, tpl } from "@/i18n";
import { common } from "@/i18n/common";
import { explainText } from "@/i18n/explain";
import { axisBase, esc, readPalette, tooltipBase } from "@/lib/charts/echarts";
import { fmtPct } from "@/lib/format";
import type { ExposureView } from "@/lib/types";

/**
 * Where breadth and exposure stand: the counts behind breadth, the exposure formula with live
 * values, the names that crossed the line since the previous plan, the breadth levels that
 * would trigger a broad trim or top-up, the holdings closest to their bands, and the history.
 */
export function ExposurePanel(props: { view: ExposureView }) {
  const t = () => explainText().exposure;
  const v = () => props.view;
  const p1 = (x: number | null | undefined) => fmtPct(x, { dp: 1 });
  const names = (list: string[]) => list.join(locale() === "zh" ? "、" : ", ");
  const pastGap = () => v().cash_gap != null && Math.abs(v().cash_gap!) > v().cash_band;
  const closest = createMemo(() => v().bands.filter((b) => b.band > 0).slice(0, 5));

  const option = createMemo(() => {
    const history = v().history;
    if (history.length < 2) return null;
    const p = readPalette();
    const labels = history.map((h) => `${h.trade_date.slice(5)} ${common().slot[h.slot]}`);
    const pct = (x: number | null) => (x == null ? null : Math.round(x * 1000) / 10);
    return {
      animation: false,
      grid: { left: 40, right: 12, top: 28, bottom: 24 },
      legend: { top: 0, right: 0, itemWidth: 14, itemHeight: 2, textStyle: { color: p.label, fontSize: 11 } },
      tooltip: {
        ...tooltipBase(p),
        trigger: "axis",
        formatter: (items: { dataIndex: number; seriesName: string; value: number | null; marker: string }[]) => {
          const h = history[items[0]?.dataIndex ?? 0];
          const rows = items.map((i) => `${i.marker}${esc(i.seriesName)} <b>${i.value == null ? "—" : `${i.value.toFixed(1)}%`}</b>`);
          return [`#${h.plan_id} · ${esc(h.trade_date)} ${esc(common().slot[h.slot])}`, ...rows].join("<br/>");
        },
      },
      xAxis: { type: "category", data: labels, ...axisBase(p), splitLine: { show: false } },
      yAxis: { type: "value", min: 0, max: 100, ...axisBase(p), axisLabel: { ...axisBase(p).axisLabel, formatter: "{value}%" } },
      series: [
        { name: t().series_breadth, type: "line", data: history.map((h) => pct(h.breadth)), symbolSize: 5, lineStyle: { width: 2, color: p.series[0] }, itemStyle: { color: p.series[0] } },
        { name: t().series_exposure, type: "line", step: "end", data: history.map((h) => pct(h.exposure_target)), symbolSize: 4, lineStyle: { width: 2, color: p.series[1] }, itemStyle: { color: p.series[1] } },
      ],
    };
  });

  return (
    <div class="card exposure-card">
      <div class="card-head">
        <div style={{ "min-width": 0 }}>
          <h2>{t().title}</h2>
          <div class="sub">
            <A href={`/plans/${v().plan_id}`}>{tpl(t().sub, { id: v().plan_id, date: v().trade_date, slot: common().slot[v().slot] })}</A>
          </div>
        </div>
      </div>
      <div class="card-body exposure-body">
        <div class="exposure-col">
        <div class="exposure-figures">
          <div class="exposure-big">
            <span class="label">{t().breadth}</span>
            <span class="value num">{p1(v().breadth)}</span>
            <Show when={v().above != null && v().counted != null}>
              <span class="muted xs">{tpl(t().counts, { above: v().above, counted: v().counted, line: v().line })}</span>
            </Show>
          </div>
          <div class="exposure-text xs">
            <p class="num">
              {v().breadth_scaling
                ? tpl(t().formula, {
                    min: v().min_exposure.toFixed(2),
                    max: v().max_exposure.toFixed(2),
                    span: (v().max_exposure - v().min_exposure).toFixed(2),
                    breadth: (v().breadth ?? 0).toFixed(3),
                    exposure: p1(v().exposure_target),
                  })
                : tpl(t().formula_off, { max: p1(v().max_exposure) })}
            </p>
            <p>
              {tpl(t().invested, {
                now: p1(v().invested_now),
                target: p1(v().invested_target),
                gap: Math.abs((v().cash_gap ?? 0) * 100).toFixed(1),
                band: fmtPct(v().cash_band, { dp: 0 }),
              })}
            </p>
            <Show when={v().crossed_below.length}>
              <p>{tpl(t().crossed_below, { names: names(v().crossed_below) })}</p>
            </Show>
            <Show when={v().crossed_above.length}>
              <p>{tpl(t().crossed_above, { names: names(v().crossed_above) })}</p>
            </Show>
            <Show when={v().previous_plan_id && !v().crossed_below.length && !v().crossed_above.length}>
              <p class="muted">{tpl(t().no_cross, { line: v().line })}</p>
            </Show>
          </div>
        </div>
        <Show when={v().triggers}>
          {(tr) => (
            <ul class="exposure-triggers xs">
              <Show when={pastGap()}>
                <li class="warn">{t().gap_past}</li>
              </Show>
              <li>
                {tr().trim_below != null
                  ? tpl(t().trim, { breadth: p1(tr().trim_below), count: tr().trim_below_count ?? "—" })
                  : t().trim_none}
              </li>
              <li>
                {tr().top_up_above != null
                  ? tpl(t().top_up, { breadth: p1(tr().top_up_above), count: tr().top_up_above_count ?? "—" })
                  : t().top_up_none}
              </li>
            </ul>
          )}
        </Show>
        <p class="muted xs">{tpl(t().eligibility, { days: v().min_history_days, line: v().line })}</p>
        </div>
        <div class="exposure-col">
        <Show when={option()}>
          <Chart option={option} height={170} ariaLabel={t().chart} />
        </Show>
        <Show when={closest().length}>
          <div>
            <div class="kicker">{t().bands}</div>
            <ul class="exposure-bands xs">
              <For each={closest()}>
                {(b) => (
                  <li>
                    <span class="ticker">{b.symbol}</span>
                    <div class="band-meter" title={tpl(t().band_row, { drift: fmtPct(b.drift, { dp: 2, sign: true }), band: fmtPct(b.band, { dp: 2 }) })}>
                      <div class="fill" style={{ width: `${Math.min(100, (Math.abs(b.drift) / b.band) * 100)}%` }} classList={{ over: Math.abs(b.drift) > b.band }} />
                    </div>
                    <span class="num muted nowrap">{tpl(t().band_row, { drift: fmtPct(b.drift, { dp: 2, sign: true }), band: fmtPct(b.band, { dp: 2 }) })}</span>
                  </li>
                )}
              </For>
            </ul>
            <p class="muted xs">{t().band_hint}</p>
          </div>
        </Show>
        </div>
      </div>
    </div>
  );
}
