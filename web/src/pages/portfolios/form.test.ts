import { describe, expect, test } from "bun:test";
import { DEFAULT_CASH, formatCash, nameProblem, parseCash } from "@/pages/portfolios/form";

describe("portfolio name", () => {
  test("is required and trimmed", () => {
    expect(nameProblem("")).toBe("required");
    expect(nameProblem("   ")).toBe("required");
    expect(nameProblem("  Steady  ")).toBeNull();
  });

  test("allows 60 characters, counting CJK and emoji as one each", () => {
    expect(nameProblem("a".repeat(60))).toBeNull();
    expect(nameProblem("a".repeat(61))).toBe("too_long");
    expect(nameProblem("稳".repeat(60))).toBeNull();
    expect(nameProblem("📈".repeat(60))).toBeNull();
    expect(nameProblem("📈".repeat(61))).toBe("too_long");
  });
});

describe("initial cash", () => {
  test("accepts separators, decimals and full-width digits", () => {
    expect(parseCash("1,000,000")).toBe(1_000_000);
    expect(parseCash("250000.5")).toBe(250_000.5);
    expect(parseCash("１２３４５")).toBe(12_345);
    expect(parseCash(" 2 000 ")).toBe(2_000);
  });

  test("enforces the server's range", () => {
    expect(parseCash("1000")).toBe(1_000);
    expect(parseCash("999.99")).toBeNull();
    expect(parseCash("10,000,000,000")).toBe(10_000_000_000);
    expect(parseCash("10000000000.01")).toBeNull();
  });

  test("rejects anything that is not a plain amount", () => {
    expect(parseCash("")).toBeNull();
    expect(parseCash("1e6")).toBeNull();
    expect(parseCash("-5000")).toBeNull();
    expect(parseCash("5000.123")).toBeNull();
    expect(parseCash("abc")).toBeNull();
  });

  test("round-trips the default through its display form", () => {
    expect(formatCash(DEFAULT_CASH)).toBe("1,000,000");
    expect(parseCash(formatCash(DEFAULT_CASH))).toBe(DEFAULT_CASH);
  });
});
