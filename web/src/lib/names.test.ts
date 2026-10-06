import { describe, expect, test } from "bun:test";
import { setLocale } from "@/i18n";
import { actorName, strategyName } from "@/lib/names";

describe("display names", () => {
  test("versions named after their preset follow the UI language", () => {
    setLocale("zh");
    expect(strategyName({ name: "Sector-first risk budget", preset_id: "sector_risk_budget" })).toBe("板块优先 · 风险预算");
    setLocale("en");
    expect(strategyName({ name: "板块优先 · 风险预算", preset_id: "sector_risk_budget" })).toBe("Sector-first risk budget");
  });

  test("custom version names are kept", () => {
    setLocale("zh");
    expect(strategyName({ name: "2026 Q4 tighter caps", preset_id: "sector_risk_budget" })).toBe("2026 Q4 tighter caps");
  });

  test("system actors are translated, people are not", () => {
    setLocale("zh");
    expect(actorName("scheduler")).toBe("自动调度");
    expect(actorName("alice")).toBe("alice");
  });
});
