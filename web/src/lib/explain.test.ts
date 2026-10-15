import { beforeEach, describe, expect, test } from "bun:test";
import { setLocale } from "@/i18n";
import { driverChip, driverText, oneLine, orderWhy } from "@/lib/explain";
import type { OrderExplanation, PlanDriver, PlanExplanation } from "@/lib/types";

// Plan #17 of the production paper account (2026-10-15 open), as explained by the server.
const drivers: PlanDriver[] = [
  { code: "breadth", line: 200, before: 0.578125, now: 0.390625, crossed_below: 12, crossed_above: 0, exposure_before: 0.8196875, exposure_now: 0.7484375, orders: 1, weight: 0.0017 },
  { code: "band_waiver", side: "sells", gap: -0.0759, invested_now: 0.8244, invested_target: 0.7484, orders: 18 },
  { code: "trend_breaks", count: 18, orders: 10, weight: 0.038 },
  { code: "sector_budget", sector: "AI-chip", before: 0.133, now: 0.11, orders: 5, weight: 0.026 },
];
const sector = (id: string) => (id === "AI-chip" ? "AI chips" : id);

function order(over: Partial<OrderExplanation>): OrderExplanation {
  return {
    symbol: "X",
    sector: "hyperscaler",
    side: "sell",
    weight_before: 0.04,
    weight_target: 0.03,
    previous_target: 0.035,
    drift: 0.005,
    model: -0.005,
    driver: "price_drift",
    status_before: "active",
    status_now: "active",
    components: [],
    price_drift: 0.005,
    left_over: null,
    left_over_cause: null,
    previous_order_status: null,
    previous_status_reason: null,
    price_change: null,
    trend_multiplier_before: 1,
    trend_multiplier_now: 1,
    band_waived: false,
    scaled: false,
    ...over,
  };
}

describe("plan explanations", () => {
  beforeEach(() => setLocale("en"));

  test("the headline reads as one sentence", () => {
    expect(oneLine(drivers, sector)).toBe(
      "Breadth 57.8%→39.1% (12 crossed below the 200-day line), target exposure 82.0%→74.8%; " +
        "invested 82.4% was 7.6 pts above the 74.8% target, past the 5-pt tolerance, so sell bands were skipped; " +
        "18 trend breaks (10 orders).",
    );
    // Acronyms keep their case in later clauses.
    expect(oneLine([drivers[2], drivers[3]], sector)).toBe("18 trend breaks (10 orders); AI chips budget 13.3%→11.0%.");
    setLocale("zh");
    expect(oneLine(drivers.slice(0, 1), sector)).toBe("市场宽度 57.8%→39.1%，12 只跌破 200 日线，目标仓位 82.0%→74.8%。");
  });

  test("counts agree in number", () => {
    expect(driverText({ code: "retry", orders: 1, weight: 0.01 }, sector)).toBe("Retrying 1 order that did not fill last plan");
    expect(driverText({ code: "trend_recoveries", count: 2, orders: 0, weight: 0 }, sector)).toBe("2 trend recoveries");
    expect(driverText({ code: "newly_eligible", count: 34, cut: 16, orders: 50, weight: 0.8 }, sector)).toBe(
      "34 names became eligible and entered, cutting 16 existing positions to share their sector budgets",
    );
  });

  test("chips are short", () => {
    expect(driverChip(drivers[0], sector)).toBe("Breadth 57.8%→39.1%");
    expect(driverChip({ code: "turnover_cap", scale: 0.68, limit: 0.3 }, sector)).toBe("Turnover cap 68%");
  });

  test("orders say what moved them", () => {
    const x = { exposure: { before: 0.82, now: 0.748 }, sectors: [{ sector: "hyperscaler", before: 0.133, now: 0.11, share_effect: -0.011 }] } as unknown as PlanExplanation;
    expect(orderWhy(order({ driver: "price_drift", price_change: -0.114, previous_target: 0.0106 }), x, sector)).toBe("Price −11.4%, drifted from last target 1.1%");
    expect(orderWhy(order({ driver: "trend", trend_multiplier_before: 0.94, trend_multiplier_now: 0.5 }), x, sector)).toBe("Trend multiplier 0.94→0.50");
    expect(orderWhy(order({ driver: "sector_budget" }), x, sector)).toBe("hyperscaler budget 13.3%→11.0%");
    expect(orderWhy(order({ driver: "retry", previous_order_status: "rejected", previous_status_reason: "price_moved" }), x, sector)).toBe("Last order rejected (price_moved): retrying");
    expect(orderWhy(order({ driver: "new_peers" }), x, sector)).toBe("Newly eligible peers share the sector budget");
  });
});
