import { beforeEach, describe, expect, test } from "bun:test";
import { setLocale } from "@/i18n";
import { fmtCountdown, fmtDual, fmtMoney, fmtPct, fmtQty, moneyPolarity, polarity, toNumber } from "@/lib/format";
import { setDisplayTz } from "@/lib/prefs";

describe("numbers", () => {
  beforeEach(() => setLocale("en"));

  test("decimal strings from the API parse", () => {
    expect(toNumber("1234.50")).toBe(1234.5);
    expect(toNumber("")).toBeNull();
    expect(toNumber(null)).toBeNull();
    expect(toNumber("abc")).toBeNull();
  });

  test("money keeps the sign of the printed value", () => {
    expect(fmtMoney(1234.5)).toBe("$1,234.50");
    expect(fmtMoney(-12.345, { dp: 2 })).toBe("−$12.35");
    expect(fmtMoney(5, { sign: true })).toBe("+$5.00");
    // −0.40 printed without decimals is "$0", never "−$0".
    expect(fmtMoney(-0.4, { dp: 0 })).toBe("$0");
    expect(fmtMoney(null)).toBe("—");
  });

  test("compact money uses 万/亿 in Chinese and K/M/B in English", () => {
    expect(fmtMoney(1_250_000, { compact: true })).toBe("$1.25M");
    setLocale("zh");
    expect(fmtMoney(1_250_000, { compact: true })).toBe("$125.0万");
    expect(fmtMoney(460_000, { compact: true })).toBe("$46.00万");
    expect(fmtMoney(320_000_000, { compact: true })).toBe("$3.20亿");
  });

  test("percentages round before choosing the sign", () => {
    expect(fmtPct(0.01234, { dp: 2, sign: true })).toBe("+1.23%");
    expect(fmtPct(-0.00004, { dp: 2, sign: true })).toBe("0.00%");
    expect(fmtPct(-0.25, { dp: 0 })).toBe("−25%");
  });

  test("polarity follows the printed value when decimals are given", () => {
    expect(polarity(0.02)).toBe("up");
    expect(polarity(-0.00004, 2)).toBe("flat");
    expect(polarity(-0.0004, 2)).toBe("down");
    expect(moneyPolarity(-0.4, 0)).toBe("flat");
    expect(moneyPolarity(-0.6, 0)).toBe("down");
  });

  test("share quantities", () => {
    expect(fmtQty("25.000000")).toBe("25");
    expect(fmtQty(0.1234567)).toBe("0.1235");
  });
});

describe("times", () => {
  beforeEach(() => {
    setLocale("en");
    setDisplayTz("Asia/Singapore");
  });

  test("dual time shows Singapore first, then New York", () => {
    expect(fmtDual("2026-10-05T14:00:00Z")).toBe("22:00 SGT · 10:00 ET");
  });

  test("auto dates the local time only when the calendar day differs", () => {
    // 13:00 ET on 5 Oct is 01:00 on 6 Oct in Singapore.
    expect(fmtDual("2026-10-05T17:00:00Z", "auto")).toBe("10-06 01:00 SGT · 13:00 ET");
    expect(fmtDual("2026-10-05T14:00:00Z", "auto")).toBe("22:00 SGT · 10:00 ET");
  });

  test("countdowns", () => {
    expect(fmtCountdown(42_000)).toBe("42s");
    expect(fmtCountdown(3_725_000)).toBe("1h 2m");
    setLocale("zh");
    expect(fmtCountdown(125_000)).toBe("2分05秒");
  });
});
