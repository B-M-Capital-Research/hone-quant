import { describe, expect, test } from "bun:test";
import type { Palette } from "@/lib/charts/echarts";
import { equityDrawdownOption, signed } from "./charts";

// Every colour token resolves to one value; only the shape of the option matters here.
const palette = new Proxy({}, { get: (_target, key) => (key === "series" ? ["#111111", "#222222"] : "#333333") }) as unknown as Palette;
const labels = { cumulative: "Cumulative", drawdown: "Drawdown", maxDd: "Max drawdown {v}" };

function build(values: number[], drawdowns: number[]) {
  const dates = values.map((_, i) => `2026-10-0${i + 1}`);
  return equityDrawdownOption(dates, [{ name: "Portfolio", label: "Portfolio", values, slot: 0, emphasis: true }], drawdowns, palette, {
    width: 1200,
    height: 400,
    log: false,
    labels,
  });
}

describe("signed", () => {
  test("a value that rounds to zero carries no sign", () => {
    expect(signed(-0.04, 1)).toBe("0.0");
    expect(signed(0.04, 1)).toBe("0.0");
    expect(signed(0, 2)).toBe("0.00");
  });

  test("other values keep their sign", () => {
    expect(signed(-0.04, 2)).toBe("−0.04");
    expect(signed(12.34, 1)).toBe("+12.3");
  });
});

describe("equity and drawdown labels", () => {
  test("a young account's small moves and shallow drawdown keep two decimals", () => {
    const option = build([1_000_000, 999_700], [0, -0.0003]);
    const ddAxis = option.yAxis[1].axisLabel.formatter;
    expect(ddAxis(0)).toBe("0%");
    expect(ddAxis(-0.02)).toBe("−0.02%");
    expect(option.series[0].endLabel.formatter({ value: -0.03 })).toBe("Portfolio −0.03%");
    expect(option.series[1].markPoint.label.formatter()).toBe("Max drawdown −0.03%");
  });

  test("a deep drawdown uses whole percents on the axis", () => {
    const option = build([100, 120, 90, 95], [0, 0, -0.25, -0.2083]);
    const ddAxis = option.yAxis[1].axisLabel.formatter;
    expect(ddAxis(-10)).toBe("−10%");
    expect(option.series[1].markPoint.label.formatter()).toBe("Max drawdown −25.0%");
    expect(option.series[0].endLabel.formatter({ value: -5 })).toBe("Portfolio −5.0%");
  });
});
