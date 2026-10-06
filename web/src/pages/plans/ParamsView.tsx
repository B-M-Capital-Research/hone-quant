import { For } from "solid-js";
import { locale } from "@/i18n";
import { fmtNum, fmtPct } from "@/lib/format";
import type { StrategyParams } from "@/lib/types";

/** Labels for the parameters shown on a plan (the Strategy page explains them in depth). */
const LABELS: Record<string, { zh: string; en: string; unit?: "pct" | "days" | "usd" | "x" | "bool" | "text" }> = {
  "universe.min_history_days": { zh: "最少历史天数", en: "Minimum history", unit: "days" },
  "exposure.max_exposure": { zh: "最高仓位", en: "Maximum invested", unit: "pct" },
  "exposure.min_exposure": { zh: "最低仓位", en: "Minimum invested", unit: "pct" },
  "exposure.breadth_scaling": { zh: "按市场宽度调节仓位", en: "Scale by breadth", unit: "bool" },
  "exposure.breadth_sma": { zh: "宽度均线", en: "Breadth average", unit: "days" },
  "exposure.target_vol": { zh: "目标波动率", en: "Volatility target", unit: "pct" },
  "exposure.vol_lookback": { zh: "波动率窗口", en: "Volatility window", unit: "days" },
  "sector.method": { zh: "板块预算方式", en: "Sector budget method", unit: "text" },
  "sector.vol_power": { zh: "波动率倒数幂次", en: "Inverse-volatility power", unit: "x" },
  "sector.vol_lookback": { zh: "波动率窗口", en: "Volatility window", unit: "days" },
  "sector.momentum_tilt": { zh: "动量倾斜", en: "Momentum tilt", unit: "x" },
  "sector.momentum_lookback": { zh: "动量窗口", en: "Momentum window", unit: "days" },
  "sector.momentum_skip": { zh: "动量剔除近期", en: "Momentum skip", unit: "days" },
  "sector.trend_aware": { zh: "趋势感知", en: "Trend-aware", unit: "bool" },
  "sector.min_weight": { zh: "板块下限", en: "Sector floor", unit: "pct" },
  "sector.max_weight": { zh: "板块上限", en: "Sector cap", unit: "pct" },
  "asset.vol_power": { zh: "波动率倒数幂次", en: "Inverse-volatility power", unit: "x" },
  "asset.vol_lookback": { zh: "波动率窗口", en: "Volatility window", unit: "days" },
  "asset.momentum_tilt": { zh: "动量倾斜", en: "Momentum tilt", unit: "x" },
  "asset.momentum_lookback": { zh: "动量窗口", en: "Momentum window", unit: "days" },
  "asset.momentum_skip": { zh: "动量剔除近期", en: "Momentum skip", unit: "days" },
  "asset.trend_sma": { zh: "趋势均线", en: "Trend average", unit: "days" },
  "asset.trend_penalty": { zh: "跌破均线时的权重系数", en: "Weight multiplier below trend", unit: "x" },
  "asset.trend_ramp": { zh: "趋势渐降区间", en: "Trend fade", unit: "pct" },
  "asset.min_weight": { zh: "单一公司下限", en: "Minimum position", unit: "pct" },
  "asset.max_weight": { zh: "单一公司上限", en: "Single-name cap", unit: "pct" },
  "rebalance.band_abs": { zh: "绝对容忍带", en: "Absolute band", unit: "pct" },
  "rebalance.band_rel": { zh: "相对容忍带", en: "Relative band", unit: "pct" },
  "rebalance.min_trade_value": { zh: "最小交易金额", en: "Minimum trade", unit: "usd" },
  "rebalance.max_turnover": { zh: "单次换手上限", en: "Max turnover per plan", unit: "pct" },
  "rebalance.fractional_shares": { zh: "允许碎股", en: "Fractional shares", unit: "bool" },
};

const GROUPS: { key: keyof StrategyParams; zh: string; en: string }[] = [
  { key: "exposure", zh: "仓位", en: "Exposure" },
  { key: "sector", zh: "板块预算", en: "Sector budgets" },
  { key: "asset", zh: "单一公司", en: "Single names" },
  { key: "rebalance", zh: "调仓", en: "Rebalancing" },
  { key: "universe", zh: "投资范围", en: "Universe" },
];

function format(value: unknown, unit: string | undefined): string {
  if (value === null || value === undefined) return locale() === "zh" ? "不启用" : "off";
  if (typeof value === "boolean") return value ? (locale() === "zh" ? "是" : "yes") : locale() === "zh" ? "否" : "no";
  if (typeof value === "number") {
    switch (unit) {
      case "pct":
        return fmtPct(value, { dp: value < 0.1 ? 1 : 0 });
      case "days":
        return locale() === "zh" ? `${value} 个交易日` : `${value} sessions`;
      case "usd":
        return `$${fmtNum(value, 0)}`;
      default:
        return fmtNum(value, 2);
    }
  }
  if (typeof value === "object") return JSON.stringify(value);
  return String(value);
}

export function ParamsView(props: { params: StrategyParams }) {
  return (
    <div class="params-grid">
      <For each={GROUPS}>
        {(group) => {
          const values = () => (props.params[group.key] ?? {}) as unknown as Record<string, unknown>;
          return (
            <div class="params-group">
              <div class="kicker">{locale() === "zh" ? group.zh : group.en}</div>
              <dl class="kv">
                <For each={Object.entries(values()).filter(([k]) => k !== "custom_budgets" || Object.keys((values()[k] as object) ?? {}).length)}>
                  {([key, value]) => {
                    const meta = LABELS[`${group.key}.${key}`];
                    return (
                      <>
                        <dt>{meta ? meta[locale()] : key}</dt>
                        <dd>{format(value, meta?.unit)}</dd>
                      </>
                    );
                  }}
                </For>
              </dl>
            </div>
          );
        }}
      </For>
    </div>
  );
}
