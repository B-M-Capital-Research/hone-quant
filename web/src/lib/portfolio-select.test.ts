import { describe, expect, test } from "bun:test";
import { eventConcerns, ownerKind, parsePortfolioId, resolveSelection, takePortfolioParam } from "@/lib/portfolio-select";

const list = [
  { id: 1, status: "active" as const },
  { id: 4, status: "active" as const },
  { id: 7, status: "archived" as const },
];

describe("portfolio selection", () => {
  test("keeps the preferred portfolio while it is active and visible", () => {
    expect(resolveSelection(list, 4, 1)).toBe(4);
  });

  test("falls back to the server default, then to the first active one", () => {
    expect(resolveSelection(list, 9, 1)).toBe(1);
    expect(resolveSelection(list, 7, 4)).toBe(4);
    expect(resolveSelection(list, null, null)).toBe(1);
    expect(resolveSelection(list, 9, 7)).toBe(1);
  });

  test("never picks an id the server rejected", () => {
    expect(resolveSelection(list, 4, 4, new Set([4]))).toBe(1);
    expect(resolveSelection(list, 1, 4, new Set([1, 4]))).toBeNull();
  });

  test("is null without any active portfolio", () => {
    expect(resolveSelection([], 3, null)).toBeNull();
    expect(resolveSelection([{ id: 2, status: "archived" }], 2, 2)).toBeNull();
  });
});

describe("portfolio ids and links", () => {
  test("parses positive integer ids only", () => {
    expect(parsePortfolioId("12")).toBe(12);
    expect(parsePortfolioId(" 3 ")).toBe(3);
    expect(parsePortfolioId("0")).toBeNull();
    expect(parsePortfolioId("-2")).toBeNull();
    expect(parsePortfolioId("1.5")).toBeNull();
    expect(parsePortfolioId("abc")).toBeNull();
    expect(parsePortfolioId("")).toBeNull();
    expect(parsePortfolioId(null)).toBeNull();
  });

  test("takes the portfolio parameter off the search string and keeps the rest", () => {
    expect(takePortfolioParam("?portfolio=3")).toEqual({ id: 3, search: "" });
    expect(takePortfolioParam("?tab=fills&portfolio=12")).toEqual({ id: 12, search: "?tab=fills" });
    expect(takePortfolioParam("?portfolio=x&range=90")).toEqual({ id: null, search: "?range=90" });
  });

  test("leaves URLs without the parameter alone", () => {
    expect(takePortfolioParam("")).toBeNull();
    expect(takePortfolioParam("?tab=fills")).toBeNull();
  });
});

describe("owners", () => {
  test("shared, own or someone else's", () => {
    expect(ownerKind({ owner: null }, "alice")).toBe("shared");
    expect(ownerKind({ owner: "alice" }, "alice")).toBe("own");
    expect(ownerKind({ owner: "bob" }, "alice")).toBe("other");
    expect(ownerKind({ owner: "bob" }, undefined)).toBe("other");
  });
});

describe("server events", () => {
  test("events about another portfolio are ignored", () => {
    expect(eventConcerns({ type: "plan", plan_id: 9, status: "executed", portfolio_id: 2 }, 3)).toBe(false);
    expect(eventConcerns({ type: "account", reason: "fill", portfolio_id: 3 }, 3)).toBe(true);
  });

  test("global events and an unknown selection always pass", () => {
    expect(eventConcerns({ type: "settings", key: "schedule", portfolio_id: null }, 3)).toBe(true);
    expect(eventConcerns({ type: "universe" }, 3)).toBe(true);
    expect(eventConcerns({ type: "resync" }, 3)).toBe(true);
    expect(eventConcerns({ type: "plan", plan_id: 9, status: "pending", portfolio_id: 2 }, null)).toBe(true);
  });
});
