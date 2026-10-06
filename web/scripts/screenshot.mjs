#!/usr/bin/env node
/**
 * Screenshots of the running web app for visual review.
 *
 *   HONE_QUANT_SHOT_PASSWORD=... node scripts/screenshot.mjs --base http://127.0.0.1:5173 \
 *     --out ./shots --pages /,/plans --locales zh,en --themes light,dark --sizes 1440x900,390x844
 *
 * Signs in through the API as HONE_QUANT_SHOT_USER (default "admin"). Console errors and page
 * exceptions are printed so they are not missed. `--full false` captures the viewport only;
 * `--clip x,y,w,h` captures a region of the full page (add `--suffix name` to keep several).
 * `--set board.view=asset,board.symbol=NVDA` presets stored UI choices (`hone-quant.<key>`).
 */
import { chromium } from "playwright";
import { mkdir } from "node:fs/promises";
import path from "node:path";

const argv = process.argv.slice(2);
const args = {};
for (let i = 0; i < argv.length; i++) {
  if (!argv[i].startsWith("--")) continue;
  const next = argv[i + 1];
  args[argv[i].slice(2)] = next === undefined || next.startsWith("--") ? "true" : next;
}

const base = (args.base ?? "http://127.0.0.1:5173").replace(/\/$/, "");
const out = args.out ?? "shots";
const pages = (args.pages ?? "/").split(",");
const locales = (args.locales ?? "zh").split(",");
const themes = (args.themes ?? "light").split(",");
const sizes = (args.sizes ?? "1440x900").split(",").map((s) => s.split("x").map(Number));
const full = args.full !== "false";
// Optional region "x,y,width,height" (page coordinates) for close-up review.
const clip = args.clip ? (([x, y, width, height]) => ({ x, y, width, height }))(args.clip.split(",").map(Number)) : undefined;
const suffix = args.suffix ? `-${args.suffix}` : "";
const presets = (args.set ? args.set.split(",") : []).map((pair) => pair.split("="));
const wait = Number(args.wait ?? 1800);
const user = process.env.HONE_QUANT_SHOT_USER ?? "admin";
const password = process.env.HONE_QUANT_SHOT_PASSWORD;
if (!password) {
  console.error("Set HONE_QUANT_SHOT_PASSWORD.");
  process.exit(2);
}

await mkdir(out, { recursive: true });
const browser = await chromium.launch();
try {
  for (const [width, height] of sizes) {
    for (const locale of locales) {
      for (const theme of themes) {
        const context = await browser.newContext({ viewport: { width, height }, deviceScaleFactor: 1 });
        await context.addInitScript(
          ([l, t, extra]) => {
            localStorage.setItem("hone-quant.locale", l);
            localStorage.setItem("hone-quant.theme", t);
            for (const [key, value] of extra) localStorage.setItem(`hone-quant.${key}`, value);
          },
          [locale, theme, presets],
        );
        const page = await context.newPage();
        page.on("console", (msg) => {
          if (msg.type() === "error") console.log(`  console error: ${msg.text()}`);
        });
        page.on("pageerror", (error) => console.log(`  page error: ${error.message}`));
        const res = await page.request.post(`${base}/api/auth/login`, {
          data: { username: user, password },
          headers: { "X-Hone-Quant-Action": "1" },
        });
        if (!res.ok()) throw new Error(`login failed: HTTP ${res.status()}`);
        for (const p of pages) {
          // The SSE stream keeps a request open, so wait for "load" plus a settle delay.
          await page.goto(base + p, { waitUntil: "load" });
          await page.waitForTimeout(wait);
          const slug = p === "/" ? "home" : p.replace(/^\//, "").replace(/[/?=&]/g, "_");
          const file = path.join(out, `${slug}-${locale}-${theme}-${width}${suffix}.png`);
          await page.screenshot({ path: file, fullPage: full || Boolean(clip), clip });
          console.log(file);
        }
        await context.close();
      }
    }
  }
} finally {
  await browser.close();
}
