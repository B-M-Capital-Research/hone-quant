import { describe, expect, test } from "bun:test";
import type { StrategyParams } from "@/lib/types";
import { diffPaths, setPath, validateParams, withDefaults } from "./params";

const defaults: StrategyParams = {
  universe: { min_history_days: 126 },
  sector: {
    method: "risk",
    vol_power: 1,
    vol_lookback: 63,
    momentum_tilt: 0.3,
    momentum_lookback: 126,
    momentum_skip: 21,
    trend_aware: true,
    min_weight: 0.03,
    max_weight: 0.22,
    custom_budgets: {},
  },
  asset: {
    vol_power: 1,
    vol_lookback: 63,
    momentum_tilt: 0.4,
    momentum_lookback: 126,
    momentum_skip: 21,
    trend_sma: 200,
    trend_penalty: 0.5,
    trend_ramp: 0.1,
    min_weight: 0.005,
    max_weight: 0.05,
  },
  exposure: { max_exposure: 0.98, min_exposure: 0.6, breadth_scaling: true, breadth_sma: 200, target_vol: null, vol_lookback: 63 },
  rebalance: { band_abs: 0.0025, band_rel: 0.25, min_trade_value: 1000, max_turnover: 0.3, fractional_shares: false },
  ai: { mode: "off", max_tilt: 0.25, min_confidence: 0.5, max_age_days: 5 },
};

describe("AI sector view parameters", () => {
  test("versions saved before the AI view load with it off", () => {
    const { ai: _, ...old } = defaults;
    const params = withDefaults(old, defaults);
    expect(params.ai?.mode).toBe("off");
    expect(diffPaths(params, defaults)).toEqual([]);
  });

  test("ranges mirror the server", () => {
    expect(validateParams(defaults)).toEqual([]);
    const live = setPath(defaults, "ai.mode", "live");
    expect(diffPaths(live, defaults)).toEqual(["ai.mode"]);
    const wide = setPath(live, "ai.max_tilt", 0.6);
    expect(validateParams(wide).map((i) => i.path)).toEqual(["ai.max_tilt"]);
    const stale = setPath(live, "ai.max_age_days", 0);
    expect(validateParams(stale).map((i) => i.path)).toEqual(["ai.max_age_days"]);
  });
});
