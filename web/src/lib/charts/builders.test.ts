import { describe, expect, test } from "bun:test";
import { aggregateBars, layoutBoard } from "@/lib/charts/builders";
import type { Board, BoardItem } from "@/lib/types";

function item(symbol: string, sector: string, change: number): BoardItem {
  return {
    symbol,
    name_zh: symbol,
    name_en: symbol,
    sector_id: sector,
    reference: 100,
    open: 100,
    high: 101,
    low: 99,
    close: 100 * (1 + change),
    volume: 1,
    open_pct: 0,
    high_pct: 0.01,
    low_pct: -0.01,
    close_pct: change,
    weight: 0.01,
    target_weight: 0.01,
    live: false,
  };
}

const sector = (id: string, sort_order: number) => ({ id, name_zh: id, name_en: id, summary_zh: "", summary_en: "", sort_order });

const board: Board = {
  period: "1D",
  from: "2026-10-05",
  to: "2026-10-05",
  live: false,
  as_of: "2026-10-05T20:00:00Z",
  sectors: [sector("chips", 0), sector("space", 1), sector("power", 2)],
  items: [item("AAA", "chips", 0.01), item("BBB", "chips", 0.03), item("CCC", "chips", -0.02), item("RKLB", "space", 0.02), item("VST", "power", 0.0)],
};

describe("layoutBoard", () => {
  test("orders sectors by the ontology and companies by period change", () => {
    const layout = layoutBoard(board, null);
    const symbols = layout.slots.filter((s) => s).map((s) => s!.symbol);
    expect(symbols).toEqual(["BBB", "AAA", "CCC", "RKLB", "VST"]);
  });

  test("separates sectors and pads narrow ones to the minimum span", () => {
    const layout = layoutBoard(board, null, { gap: 1, minSpan: 3 });
    expect(layout.spans.map((s) => [s.sector.id, s.start, s.count, s.members])).toEqual([
      ["chips", 0, 3, 3],
      ["space", 4, 3, 1],
      ["power", 8, 3, 1],
    ]);
    // Gap columns between sectors and padding around single-company sectors are empty.
    expect(layout.slots[3]).toBeNull();
    expect(layout.slots[4]).toBeNull();
    expect(layout.slots[5]?.symbol).toBe("RKLB");
    expect(layout.slots.length).toBe(11);
  });

  test("averages the period change per sector and honours the sector filter", () => {
    const layout = layoutBoard(board, new Set(["chips"]));
    expect(layout.spans).toHaveLength(1);
    expect(layout.spans[0].avg).toBeCloseTo((0.01 + 0.03 - 0.02) / 3, 10);
  });
});

describe("aggregateBars", () => {
  // Thu 2026-01-29 … Tue 2026-02-03: two weeks (Monday starts), two months.
  const bars = [
    { t: "2026-01-29", o: 10, h: 12, l: 9, c: 11, v: 100 },
    { t: "2026-01-30", o: 11, h: 13, l: 10, c: 12, v: 200 },
    { t: "2026-02-02", o: 12, h: 15, l: 11, c: 14, v: 300 },
    { t: "2026-02-03", o: 14, h: 14, l: 8, c: 9, v: 400 },
  ];
  const sma50 = [1, 2, 3, 4];
  const sma200 = [null, null, 5, 6];

  test("weekly candles start on Monday and carry the period's open, extremes, close and volume", () => {
    const w = aggregateBars(bars, sma50, sma200, "week");
    expect(w.interval).toBe("1week");
    expect(w.bars).toEqual([
      { t: "2026-01-29", o: 10, h: 13, l: 9, c: 12, v: 300 },
      { t: "2026-02-02", o: 12, h: 15, l: 8, c: 9, v: 700 },
    ]);
    // Moving averages are read at each period's last session.
    expect(w.sma50).toEqual([2, 4]);
    expect(w.sma200).toEqual([null, 6]);
  });

  test("monthly candles split at the calendar month", () => {
    const m = aggregateBars(bars, sma50, sma200, "month");
    expect(m.interval).toBe("1month");
    expect(m.bars.map((b) => [b.t, b.o, b.h, b.l, b.c, b.v])).toEqual([
      ["2026-01-29", 10, 13, 9, 12, 300],
      ["2026-02-02", 12, 15, 8, 9, 700],
    ]);
  });

  test("the input bars are not modified", () => {
    const copy = structuredClone(bars);
    aggregateBars(bars, sma50, sma200, "week");
    expect(bars).toEqual(copy);
  });
});
