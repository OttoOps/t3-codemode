async (page) => {
  const base = "__BOARD_URL__";
  const empty = "__EMPTY_URL__";
  const out = "__CAPTURE_DIR__";
  await page.addInitScript(() => {
    const ES = window.EventSource; window.__es = [];
    window.EventSource = function (...a) { const e = new ES(...a); window.__es.push(e); e.addEventListener("state", () => { window.__states = (window.__states || 0) + 1; }); return e; };
    window.EventSource.prototype = ES.prototype;
  });
  const blank = () => page.evaluate(() => {
    document.title = "";
    for (const h of document.querySelectorAll("h1")) h.textContent = "";
  });
  const shot = async (name) => { await blank(); await page.screenshot({ path: `${out}/${name}.png` }); };
  const open = async (url, w, h, scheme, wait = true) => {
    await page.emulateMedia({ colorScheme: scheme, reducedMotion: "reduce" });
    await page.setViewportSize({ width: w, height: h });
    await page.goto(url);
    if (wait) await page.waitForFunction(() => (window.__states || 0) > 0, null, { timeout: 20000 });
    await page.waitForTimeout(1200);
  };

  await open(base, 1440, 900, "light");
  await shot("1440-light");
  await open(base, 1440, 900, "dark");
  await shot("1440-dark");
  await open(base, 1280, 800, "light");
  await shot("1280-light");
  await open(base, 1024, 768, "light");
  await shot("1024-light");
  await open(base, 390, 844, "light");
  await shot("390-light");
  await open(base, 390, 844, "dark");
  await shot("390-dark");

  await open(empty, 1440, 900, "light");
  await shot("state-empty");
  await page.route("**/api/state", () => {});
  await page.route("**/api/events", () => {});
  await open(base, 1440, 900, "light", false);
  await shot("state-loading");
  await page.unroute("**/api/state");
  await page.unroute("**/api/events");
  await open(base, 1440, 900, "light");
  await page.route("**/api/events", (r) => r.abort());
  await page.evaluate(() => window.__es.forEach((e) => { e.close(); e.dispatchEvent(new Event("error")); }));
  await page.waitForTimeout(2500);
  await shot("state-disconnected");
  await page.unroute("**/api/events");

  await open(base, 1440, 900, "light");
  const field = page.locator("#issues");
  await field.scrollIntoViewIfNeeded();
  await field.click();
  await page.keyboard.type("acme/widgets#52");
  await shot("live-typing-t0");
  await page.waitForTimeout(30000);
  await shot("live-typing-t30");
  await page.keyboard.press("Escape");
  await page.evaluate(() => { document.activeElement.blur(); scrollTo(0, 0); });

  await open(base, 1440, 900, "light");
  const opt = page.locator(".card button").nth(1);
  await opt.scrollIntoViewIfNeeded();
  const box = await opt.boundingBox();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await shot("live-hover-t0");
  await page.waitForTimeout(30000);
  await shot("live-hover-t30");
  await page.mouse.move(5, 5);

  await open(base, 1440, 900, "light");
  const full = page.locator('[data-part="fullbtn"], .full summary').first();
  await full.click();
  await page.waitForTimeout(800);
  await shot("detail-open");
  return "ok";
}
