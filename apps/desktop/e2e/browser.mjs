import { chromium } from "@playwright/test";
import { mkdir } from "node:fs/promises";

export const BASE_URL = process.env.JNWS_BASE_URL ?? "http://localhost:5173";

export async function openBrowser() {
  if (process.env.JNWS_CDP) {
    return chromium.connectOverCDP(process.env.JNWS_CDP);
  }
  try {
    return await chromium.launch({ channel: process.env.JNWS_CHANNEL ?? "msedge", headless: true });
  } catch (error) {
    const message = String(error);
    if (!message.includes("EPERM") && !message.includes("Remote debugging")) throw error;
    console.warn("[e2e] direct launch blocked; connecting to CDP at http://127.0.0.1:9222");
    return chromium.connectOverCDP("http://127.0.0.1:9222");
  }
}

export async function newPage(browser, { width = 1280, height = 800, locale = "zh-CN" } = {}) {
  let context;
  try {
    context = await browser.newContext({
      viewport: { width, height },
      locale,
      colorScheme: "light",
      reducedMotion: "reduce",
    });
  } catch {
    context = browser.contexts()[0];
    if (!context) throw new Error("no browser context available");
  }
  const page = await context.newPage();
  const errors = [];
  const warnings = [];
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(message.text());
    if (message.type() === "warning") warnings.push(message.text());
  });
  page.on("pageerror", (error) => errors.push(`PAGEERROR ${error.message}`));
  return { context, page, errors, warnings };
}

export async function goto(page, query, route = "") {
  const url = `${BASE_URL}/?${query}${route ? `#/${route}` : ""}`;
  await page.goto(url, { waitUntil: "domcontentloaded" });
  await page.waitForSelector(".app-shell", { timeout: 15000 });
  await page.waitForTimeout(250);
}

export function filterExpectedConsoleErrors(errors) {
  return errors.filter((message) => {
    if (message.includes("favicon.ico")) return false;
    if (message.includes("WebGL") && message.includes("performance")) return false;
    return true;
  });
}

export async function ensureDir(path) {
  await mkdir(path, { recursive: true });
}
