import { A } from "@solidjs/router";
import { For, type JSX, Show } from "solid-js";
import { Icon } from "@/components/Icon";
import { tpl } from "@/i18n";
import { strategyText } from "@/i18n/strategy";
import { fmtMoney } from "@/lib/format";
import type { ScheduleSettings, StrategyParams } from "@/lib/types";
import { compact, tiltRange } from "./params";
import { fmtParam, methodText, minutesText, pctText, powerText, ppText } from "./format";

interface Step {
  title: string;
  body: string[];
  pills: { label: string; value: string }[];
  extra?: JSX.Element;
}

/** Plain-language walkthrough of the engine, with every number taken from `params`. */
export function Methodology(props: { params: StrategyParams; schedule: ScheduleSettings | null }) {
  const t = strategyText;
  const steps = (): Step[] => {
    const h = t().how;
    const p = props.params;
    const e = p.exposure;
    const s = p.sector;
    const a = p.asset;
    const r = p.rebalance;
    const label = (path: string) => t().fields[path as keyof ReturnType<typeof t>["fields"]].label;
    const pill = (path: string, value: unknown) => ({ label: label(path), value: fmtParam(path, value) });

    const s1: string[] = [
      e.breadth_scaling
        ? tpl(h.s1_breadth, { sma: e.breadth_sma, min: pctText(e.min_exposure), max: pctText(e.max_exposure) })
        : tpl(h.s1_fixed, { max: pctText(e.max_exposure) }),
      e.target_vol != null ? tpl(h.s1_vol_on, { vol: pctText(e.target_vol), lookback: e.vol_lookback }) : h.s1_vol_off,
    ];

    const sTilt = tiltRange(s.momentum_tilt);
    const s2: string[] = [
      s.method === "risk"
        ? tpl(h.s2_risk, { lookback: s.vol_lookback, power: powerText(s.vol_power) })
        : s.method === "member_count"
          ? h.s2_member
          : h.s2_custom,
      s.momentum_tilt > 0
        ? tpl(h.s2_momentum, { lookback: s.momentum_lookback, skip: s.momentum_skip, hi: sTilt.hi, lo: sTilt.lo })
        : h.s2_no_momentum,
    ];
    if (s.trend_aware && a.trend_sma > 0) s2.push(h.s2_trend);
    s2.push(tpl(h.s2_bounds, { floor: pctText(s.min_weight), cap: pctText(s.max_weight) }));

    const aTilt = tiltRange(a.momentum_tilt);
    const s3: string[] = [
      a.momentum_tilt > 0
        ? tpl(h.s3_body, {
            lookback: a.vol_lookback,
            power: powerText(a.vol_power),
            mlookback: a.momentum_lookback,
            skip: a.momentum_skip,
            hi: aTilt.hi,
            lo: aTilt.lo,
          })
        : tpl(h.s3_body_no_tilt, { lookback: a.vol_lookback, power: powerText(a.vol_power) }),
      a.trend_sma > 0
        ? a.trend_penalty === 0
          ? tpl(h.s3_trend_exit, { sma: a.trend_sma, ramp: pctText(a.trend_ramp) })
          : tpl(h.s3_trend, { sma: a.trend_sma, penalty: compact(a.trend_penalty, 2), ramp: pctText(a.trend_ramp) })
        : h.s3_trend_off,
      tpl(h.s3_caps, { cap: pctText(a.max_weight), min: pctText(a.min_weight), hist: p.universe.min_history_days }),
    ];

    const s4: string[] = [
      tpl(h.s4_bands, { abs: ppText(r.band_abs), rel: pctText(r.band_rel) }),
      tpl(h.s4_turnover, { turnover: pctText(r.max_turnover) }),
      tpl(h.s4_trades, { min: fmtMoney(r.min_trade_value, { dp: 0 }), shares: r.fractional_shares ? h.s4_fractional : h.s4_whole }),
    ];

    const sched = props.schedule;
    const s5: string[] = [
      tpl(h.s5_body, {
        open: minutesText(sched?.open_offset_minutes ?? 30),
        close: minutesText(sched?.close_offset_minutes ?? 180),
        review: minutesText(sched?.review_minutes ?? 10),
      }),
    ];

    return [
      {
        title: h.s1_title,
        body: s1,
        pills: [
          pill("exposure.min_exposure", e.min_exposure),
          pill("exposure.max_exposure", e.max_exposure),
          pill("exposure.breadth_sma", e.breadth_sma),
          pill("exposure.target_vol", e.target_vol),
        ],
      },
      {
        title: h.s2_title,
        body: s2,
        pills: [
          { label: label("sector.method"), value: methodText(s.method) },
          pill("sector.momentum_tilt", s.momentum_tilt),
          pill("sector.min_weight", s.min_weight),
          pill("sector.max_weight", s.max_weight),
        ],
      },
      {
        title: h.s3_title,
        body: s3,
        pills: [
          pill("asset.vol_power", a.vol_power),
          pill("asset.momentum_tilt", a.momentum_tilt),
          pill("asset.trend_sma", a.trend_sma),
          pill("asset.max_weight", a.max_weight),
          pill("asset.min_weight", a.min_weight),
        ],
      },
      {
        title: h.s4_title,
        body: s4,
        pills: [
          pill("rebalance.band_abs", r.band_abs),
          pill("rebalance.band_rel", r.band_rel),
          pill("rebalance.max_turnover", r.max_turnover),
          pill("rebalance.min_trade_value", r.min_trade_value),
        ],
      },
      {
        title: h.s5_title,
        body: s5,
        pills: [],
        extra: (
          <A class="st-inline-link" href="/settings">
            <Icon name="settings" size={13} /> {h.s5_link}
          </A>
        ),
      },
    ];
  };

  return (
    <ol class="st-steps">
      <For each={steps()}>
        {(step, i) => (
          <li class="st-step">
            <span class="st-step-num" aria-hidden="true">
              {i() + 1}
            </span>
            <div class="st-step-main">
              <h3>{step.title}</h3>
              <For each={step.body}>{(line) => <p>{line}</p>}</For>
              <Show when={step.pills.length || step.extra}>
                <div class="pill-list st-step-pills">
                  <For each={step.pills}>
                    {(pill) => (
                      <span class="param">
                        <span class="muted">{pill.label}</span>
                        <b>{pill.value}</b>
                      </span>
                    )}
                  </For>
                  {step.extra}
                </div>
              </Show>
            </div>
          </li>
        )}
      </For>
    </ol>
  );
}
