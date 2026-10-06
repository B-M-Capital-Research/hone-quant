import { type Page, expect, test } from "@playwright/test";

const user = process.env.HONE_QUANT_E2E_USER ?? "admin";
const password = process.env.HONE_QUANT_E2E_PASSWORD ?? "";

async function useLocale(page: Page, locale: "zh" | "en") {
  await page.addInitScript((value) => localStorage.setItem("hone-quant.locale", value), locale);
}

async function signIn(page: Page) {
  const response = await page.request.post("/api/auth/login", {
    data: { username: user, password },
    headers: { "X-Hone-Quant-Action": "1" },
  });
  expect(response.ok(), "sign-in (set HONE_QUANT_E2E_PASSWORD)").toBeTruthy();
}

/** Collects page errors and console errors while a test runs. */
function watchErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(`page error: ${error.message}`));
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(`console: ${message.text()}`);
  });
  return errors;
}

test("signing in through the form opens the overview", async ({ page }) => {
  await useLocale(page, "en");
  await page.goto("/");
  await expect(page).toHaveURL(/\/login/);
  await page.getByLabel("Username").fill(user);
  await page.getByLabel("Password").fill(password);
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page).toHaveURL(/\/$/);
  await expect(page.getByRole("heading", { name: "Markets" })).toBeVisible();
  await expect(page.getByText("Net asset value")).toBeVisible();
});

test("the language switch changes the whole interface", async ({ page }) => {
  await useLocale(page, "en");
  await signIn(page);
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Markets" })).toBeVisible();
  await page.getByRole("button", { name: "Display" }).click();
  await page.getByRole("button", { name: "中文" }).click();
  await expect(page.getByRole("heading", { name: "市场行情" })).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("lang", "zh-CN");
});

const PAGES = ["/", "/plans", "/trades", "/strategy", "/universe", "/backtests", "/performance", "/notifications", "/audit", "/settings"];

for (const locale of ["zh", "en"] as const) {
  test(`every page renders without errors (${locale})`, async ({ page }) => {
    await useLocale(page, locale);
    await signIn(page);
    const errors = watchErrors(page);
    for (const path of PAGES) {
      await page.goto(path);
      await page.waitForLoadState("load");
      await expect(page.locator("main.content")).toBeVisible();
      // Give data requests time to settle; spinners must not be the final state.
      await page.waitForTimeout(1200);
      await expect(page.locator("main.content .loading-row")).toHaveCount(0, { timeout: 15_000 });
    }
    expect(errors).toEqual([]);
  });
}

test("a plan opens from the plans list", async ({ page }) => {
  await useLocale(page, "en");
  await signIn(page);
  await page.goto("/plans");
  // Wait for either the plans table or the empty state before deciding.
  await page.locator("main.content table tbody tr, main.content .empty").first().waitFor({ timeout: 15_000 });
  const first = page.locator("main.content table tbody tr").first();
  test.skip((await first.count()) === 0, "no plans yet");
  await first.click();
  await expect(page).toHaveURL(/\/plans\/\d+/);
  await expect(page.getByRole("tab", { name: /Orders/ })).toBeVisible();
});
