import assert from "node:assert/strict";
import { BASE_URL, filterExpectedConsoleErrors, goto, newPage, openBrowser } from "./browser.mjs";

const results = [];
const errors = [];

async function check(name, fn) {
  try {
    await fn();
    results.push({ name, ok: true });
    console.log(`PASS ${name}`);
  } catch (error) {
    results.push({ name, ok: false, error: String(error?.stack ?? error) });
    errors.push(error);
    console.error(`FAIL ${name}: ${error?.message ?? error}`);
  }
}

const browser = await openBrowser();

await check("connection dashboard renders live stats", async () => {
  const { page, errors: consoleErrors } = await newPage(browser);
  await goto(page, "scenario=connected&renderer=2d&lang=zh-CN", "link");
  await page.waitForFunction(() => {
    const node = document.querySelector("[data-testid=stat-frames] .stat-card__value");
    return node && node.textContent && node.textContent.replace(/\D/g, "") !== "0";
  }, undefined, { timeout: 8000 });
  assert.match(await page.locator(".topbar").innerText(), /已连接/);
  const controlText = await page.locator(".link-control").innerText();
  assert.match(controlText, /USB CDC/);
  assert.match(controlText, /BLE/);
  assert.doesNotMatch(controlText, /串口/);
  assert.doesNotMatch(controlText, /模拟链路/);
  assert.equal(filterExpectedConsoleErrors(consoleErrors).length, 0, consoleErrors.join("\n"));
  await page.context().close();
});

await check("resizable panels support drag, keyboard, and persistence", async () => {
  const { page, errors: consoleErrors } = await newPage(browser);
  await goto(page, "scenario=connected&renderer=2d&lang=zh-CN", "link");
  const layout = page.locator(".link-layout");
  const before = Number.parseFloat(await layout.evaluate((el) => getComputedStyle(el).getPropertyValue("--split-width")));
  const handle = page.locator(".link-layout .resizable-panel > .split-handle");
  const box = await handle.boundingBox();
  await page.mouse.move(box.x + box.width / 2, box.y + 60);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2 + 48, box.y + 60, { steps: 6 });
  await page.mouse.up();
  await page.waitForTimeout(200);
  const after = Number.parseFloat(await layout.evaluate((el) => getComputedStyle(el).getPropertyValue("--split-width")));
  assert.ok(after >= before + 40, `expected width to grow, got ${before} -> ${after}`);
  await handle.focus();
  await page.keyboard.press("ArrowLeft");
  await page.waitForTimeout(150);
  const keyboard = Number.parseFloat(await layout.evaluate((el) => getComputedStyle(el).getPropertyValue("--split-width")));
  assert.ok(keyboard < after, "keyboard should shrink the panel");
  await page.reload({ waitUntil: "domcontentloaded" });
  await page.waitForSelector(".app-shell");
  const persisted = Number.parseFloat(await page.locator(".link-layout").evaluate((el) => getComputedStyle(el).getPropertyValue("--split-width")));
  assert.equal(persisted, keyboard);
  assert.equal(filterExpectedConsoleErrors(consoleErrors).length, 0, consoleErrors.join("\n"));
  await page.context().close();
});

await check("key capture records real key combinations and sequences", async () => {
  const { page, errors: consoleErrors } = await newPage(browser);
  await goto(page, "scenario=connected&renderer=2d&lang=zh-CN", "keys");
  await page.locator("[data-testid=key-K7]").click();
  await page.waitForSelector("[data-testid=key-panel]");
  await page.locator(".macro-keybtn").first().click();
  await page.waitForSelector(".key-capture");
  await page.keyboard.down("Control");
  await page.keyboard.press("KeyS");
  await page.keyboard.up("Control");
  await page.waitForTimeout(200);
  assert.deepEqual(await page.locator(".macro-keybtn").allTextContents(), ["Ctrl+S", "Ctrl+V"]);
  await page.locator("[data-testid=macro-add-step]").click();
  await page.waitForSelector(".key-capture");
  await page.keyboard.press("F5");
  await page.waitForTimeout(200);
  assert.equal(await page.locator(".macro-keybtn").count(), 3);
  await page.locator("[data-testid=macro-record]").click();
  await page.waitForSelector(".key-capture");
  await page.keyboard.press("KeyA");
  await page.keyboard.down("Control");
  await page.keyboard.press("KeyC");
  await page.keyboard.up("Control");
  await page.waitForTimeout(150);
  await page.locator("[data-testid=capture-done]").click();
  await page.waitForTimeout(250);
  assert.deepEqual(await page.locator(".macro-keybtn").allTextContents(), ["Ctrl+S", "Ctrl+V", "F5", "A", "Ctrl+C"]);
  assert.equal(filterExpectedConsoleErrors(consoleErrors).length, 0, consoleErrors.join("\n"));
  await page.context().close();
});

await check("long log view supports virtual scrolling, pause, and filtering", async () => {
  const { page, errors: consoleErrors } = await newPage(browser);
  await goto(page, "scenario=long-logs&renderer=2d&lang=zh-CN", "logs");
  await page.waitForSelector("[data-testid=log-row]");
  const shown = await page.locator(".log-search__input").isVisible();
  assert.equal(shown, true);
  const bodyText = await page.locator("[data-testid=log-count]").innerText();
  assert.match(bodyText, /5000/);
  await page.getByRole("button", { name: "暂停" }).click();
  await page.locator(".log-search__input").fill("CRC");
  await page.waitForTimeout(300);
  const rows = page.locator("[data-testid=log-row]");
  assert.ok((await rows.count()) > 0);
  assert.match(await rows.first().innerText(), /CRC|校验/);
  await page.getByRole("button", { name: "继续" }).click();
  assert.equal(filterExpectedConsoleErrors(consoleErrors).length, 0, consoleErrors.join("\n"));
  await page.context().close();
});

await check("key mapping selects keys, detects conflict, and applies", async () => {
  const { page, errors: consoleErrors } = await newPage(browser);
  await goto(page, "scenario=connected&renderer=2d&lang=zh-CN", "keys");
  await page.locator("[data-testid=key-K7]").click();
  await page.waitForSelector("[data-testid=key-panel]");
  assert.match(await page.locator("[data-testid=key-panel]").innerText(), /K7/);
  await page.getByRole("radio", { name: /上位机接管/ }).click();
  await page.waitForSelector(".banner--danger");
  assert.match(await page.locator(".banner--danger").innerText(), /设备宏/);
  await page.getByRole("radio", { name: /设备宏/ }).click();
  await page.locator("[data-testid=key-apply]").click();
  await page.waitForSelector(".toast");
  assert.match(await page.locator(".toast").innerText(), /已配置并生效/);
  assert.equal(filterExpectedConsoleErrors(consoleErrors).length, 0, consoleErrors.join("\n"));
  await page.context().close();
});

await check("key mapping keyboard navigation moves selection", async () => {
  const { page } = await newPage(browser);
  await goto(page, "scenario=connected&renderer=2d&lang=zh-CN", "keys");
  await page.locator("[data-testid=key-K0]").focus();
  await page.keyboard.press("ArrowRight");
  await page.waitForTimeout(150);
  assert.equal(await page.locator("[data-testid=key-K1]").getAttribute("aria-pressed"), "true");
  assert.match(await page.locator("[data-testid=key-panel]").innerText(), /K1/);
  await page.keyboard.press("Escape");
  await page.waitForTimeout(100);
  assert.equal(await page.locator("[data-testid=key-panel]").count(), 0);
  await page.context().close();
});

await check("voice pipeline reaches a result", async () => {
  const { page, errors: consoleErrors } = await newPage(browser);
  await goto(page, "scenario=default&renderer=2d&lang=zh-CN", "audio");
  await page.getByRole("button", { name: "启用语音" }).click();
  await page.getByRole("button", { name: "模拟一次按键" }).click();
  await page.waitForSelector("[data-testid=voice-text]", { timeout: 10000 });
  assert.ok((await page.locator("[data-testid=voice-text]").innerText()).length > 8);
  assert.equal(filterExpectedConsoleErrors(consoleErrors).length, 0, consoleErrors.join("\n"));
  await page.context().close();
});

await check("language switch is immediate and complete", async () => {
  const { page, errors: consoleErrors } = await newPage(browser);
  await goto(page, "scenario=default&renderer=2d&lang=zh-CN", "settings");
  await page.getByRole("button", { name: "EN" }).first().click();
  await page.waitForTimeout(200);
  assert.match(await page.locator(".sidebar").innerText(), /Connection/);
  assert.match(await page.locator(".page-title").first().innerText(), /Settings/);
  assert.equal(await page.locator("html").getAttribute("lang"), "en-US");
  await page.getByRole("button", { name: "中" }).first().click();
  await page.waitForTimeout(150);
  assert.match(await page.locator(".sidebar").innerText(), /连接/);
  assert.equal(filterExpectedConsoleErrors(consoleErrors).length, 0, consoleErrors.join("\n"));
  await page.context().close();
});

await check("offline device saves locally with a clear badge", async () => {
  const { page, errors: consoleErrors } = await newPage(browser);
  await goto(page, "scenario=offline&renderer=2d&lang=zh-CN", "keys");
  assert.match(await page.locator(".banner--warning").first().innerText(), /设备离线/);
  await page.locator("[data-testid=key-K7]").click();
  const apply = page.getByRole("button", { name: "仅保存本机" });
  await apply.click();
  await page.waitForSelector(".toast");
  assert.match(await page.locator(".toast").innerText(), /已保存到本机/);
  assert.equal(filterExpectedConsoleErrors(consoleErrors).length, 0, consoleErrors.join("\n"));
  await page.context().close();
});

await check("3D stage renders a canvas and DOM labels", async () => {
  const { page } = await newPage(browser);
  await goto(page, "scenario=connected&renderer=3d&lang=zh-CN", "keys");
  await page.waitForSelector(".keypad-canvas", { timeout: 15000 });
  await page.waitForTimeout(1200);
  assert.equal(await page.locator(".keypad-canvas").count(), 1);
  assert.equal(await page.locator(".key3d-label").count(), 16);
  await page.locator('[data-testid="key-K7"]').click();
  await page.waitForTimeout(300);
  assert.match(await page.locator("[data-testid=key-panel]").innerText(), /K7/);
  await page.context().close();
});

await browser.close();

const failed = results.filter((result) => !result.ok);
console.log(JSON.stringify({ baseUrl: BASE_URL, total: results.length, failed: failed.length, results }, null, 2));
if (failed.length > 0) process.exit(1);
