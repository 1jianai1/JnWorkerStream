import { mkdir, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { filterExpectedConsoleErrors, goto, newPage, openBrowser } from "./browser.mjs";

const OUT_DIR = resolve(process.cwd(), "../../docs/screenshots/p1");
const SIZES = [
  { width: 1280, height: 800 },
  { width: 1920, height: 1080 },
];
const LOCALES = ["zh-CN", "en-US"];

const cases = [
  {
    page: "link",
    state: "disconnected",
    query: "scenario=default&renderer=2d",
  },
  {
    page: "link",
    state: "connected",
    query: "scenario=connected&renderer=2d",
  },
  {
    page: "link",
    state: "error",
    query: "scenario=link-error&renderer=2d",
  },
  {
    page: "link",
    state: "reconnecting",
    query: "scenario=reconnecting&renderer=2d",
  },
  {
    page: "logs",
    state: "long",
    query: "scenario=long-logs&renderer=2d",
    route: "logs",
    waitFor: "[data-testid=log-row]",
  },
  {
    page: "logs",
    state: "filtered",
    query: "scenario=long-logs&renderer=2d",
    route: "logs",
    waitFor: "[data-testid=log-row]",
    async setup(page) {
      await page.locator(".log-search__input").fill("CRC");
      await page.waitForTimeout(400);
    },
  },
  {
    page: "logs",
    state: "empty",
    query: "scenario=empty&renderer=2d",
    route: "logs",
  },
  {
    page: "audio",
    state: "idle",
    query: "scenario=default&renderer=2d",
    route: "audio",
  },
  {
    page: "audio",
    state: "done",
    query: "scenario=voice-done&renderer=2d",
    route: "audio",
    waitFor: "[data-testid=voice-text]",
  },
  {
    page: "audio",
    state: "failed",
    query: "scenario=voice-failed&renderer=2d",
    route: "audio",
  },
  {
    page: "settings",
    state: "default",
    query: "scenario=default&renderer=2d",
    route: "settings",
  },
  {
    page: "settings",
    state: "corrupt",
    query: "scenario=corrupt-settings&renderer=2d",
    route: "settings",
  },
  {
    page: "keys",
    state: "unselected",
    query: "scenario=connected&renderer=2d",
    route: "keys",
  },
  {
    page: "keys",
    state: "selected",
    query: "scenario=connected&renderer=2d",
    route: "keys",
    async setup(page) {
      await page.locator("[data-testid=key-K7]").click();
      await page.waitForSelector("[data-testid=key-panel]");
    },
  },
  {
    page: "keys",
    state: "conflict",
    query: "scenario=connected&renderer=2d",
    route: "keys",
    async setup(page) {
      await page.locator("[data-testid=key-K7]").click();
      await page.getByRole("radio", { name: /上位机接管|Host takeover/ }).click();
      await page.waitForSelector(".banner--danger");
    },
  },
  {
    page: "keys",
    state: "offline",
    query: "scenario=offline&renderer=2d",
    route: "keys",
    async setup(page) {
      await page.locator("[data-testid=key-K7]").click();
      await page.waitForSelector("[data-testid=key-panel]");
    },
  },
  {
    page: "keys",
    state: "capture",
    query: "scenario=connected&renderer=2d",
    route: "keys",
    async setup(page) {
      await page.locator("[data-testid=key-K7]").click();
      await page.waitForSelector("[data-testid=key-panel]");
      await page.locator(".macro-keybtn").first().click();
      await page.waitForSelector(".key-capture");
    },
  },
  {
    page: "keys",
    state: "3d",
    query: "scenario=connected&renderer=3d",
    route: "keys",
    waitFor: ".keypad-canvas",
  },
  {
    page: "keys",
    state: "3d-selected",
    query: "scenario=connected&renderer=3d",
    route: "keys",
    waitFor: ".keypad-canvas",
    async setup(page) {
      await page.locator("[data-testid=key-K7]").click();
      await page.waitForSelector("[data-testid=key-panel]");
    },
  },
];

const browser = await openBrowser();
await mkdir(OUT_DIR, { recursive: true });
const manifest = [];
const consoleErrors = [];

for (const locale of LOCALES) {
  for (const size of SIZES) {
    const { context, page, errors } = await newPage(browser, {
      width: size.width,
      height: size.height,
      locale,
    });
    for (const item of cases) {
      const query = `${item.query}&lang=${locale}`;
      await goto(page, query, item.route ?? "");
      if (item.waitFor) {
        await page.waitForSelector(item.waitFor, { timeout: 15000 }).catch(() => {});
      }
      await item.setup?.(page);
      if (item.state === "3d") await page.waitForTimeout(1500);
      if (item.state === "3d-selected") await page.waitForTimeout(1500);
      const file = `${item.page}-${item.state}-${size.width}x${size.height}-${locale}.png`;
      await page.screenshot({ path: resolve(OUT_DIR, file) });
      manifest.push({ file, page: item.page, state: item.state, width: size.width, height: size.height, locale });
      console.log(`SHOT ${file}`);
    }
    consoleErrors.push(...filterExpectedConsoleErrors(errors));
    await context.close();
  }
}

// Compact layout evidence (ADR-019): stacked panels, no horizontal scroll.
{
  const { context, page, errors } = await newPage(browser, { width: 1024, height: 700, locale: "zh-CN" });
  for (const [route, state] of [["link", "link"], ["keys", "keys"]]) {
    await goto(page, `scenario=connected&renderer=2d&lang=zh-CN`, route);
    const file = `layout-compact-${state}-1024x700-zh-CN.png`;
    await page.screenshot({ path: resolve(OUT_DIR, file) });
    manifest.push({ file, page: route, state: "compact", width: 1024, height: 700, locale: "zh-CN" });
    console.log(`SHOT ${file}`);
  }
  consoleErrors.push(...filterExpectedConsoleErrors(errors));
  await context.close();
}

await browser.close();
await writeFile(resolve(OUT_DIR, "manifest.json"), JSON.stringify({ generatedAt: new Date().toISOString(), cases: manifest, consoleErrors }, null, 2), "utf8");
console.log(JSON.stringify({ total: manifest.length, consoleErrors }, null, 2));
if (consoleErrors.length > 0) process.exit(1);
