async (page) => {
  const base = "__BOARD_URL__";
  const r = { a: {}, b: {}, c: {} };
  await page.addInitScript(() => {
    const ES = window.EventSource; window.__es = [];
    window.EventSource = function (...a) { const e = new ES(...a); window.__es.push(e); e.addEventListener("state", () => { window.__states = (window.__states || 0) + 1; }); return e; };
    window.EventSource.prototype = ES.prototype;
  });
  await page.emulateMedia({ colorScheme: "light", reducedMotion: "reduce" });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(base);
  await page.waitForSelector("#cards .card");
  await page.waitForTimeout(1000);
  const csrf = await page.evaluate(() => CSRF);

  const answerOther = async () => {
    const id = await page.evaluate(() => {
      const mine = document.activeElement.closest && document.activeElement.closest(".card");
      const c = [...document.querySelectorAll("#cards .card:not(.gone)")].find((el) => el !== mine && el.querySelector("[data-opt]"));
      return c && c.dataset.id;
    });
    if (!id) return null;
    const res = await page.request.post(`${base}api/cards/${id}/answer`, { headers: { "X-T3CM-CSRF": csrf }, data: { text: "merge" } });
    return { id, status: res.status() };
  };
  const cut = async () => {
    await page.route("**/api/events", (route) => route.abort());
    await page.evaluate(() => { const e = window.__es[window.__es.length - 1]; e.close(); e.dispatchEvent(new Event("error")); });
    await page.waitForTimeout(5000);
    const s = await page.evaluate(() => window.__states || 0);
    await page.unroute("**/api/events");
    await page.waitForFunction((n) => (window.__states || 0) > n, s, { timeout: 40000 });
  };
  const probe = () => page.evaluate(() => {
    const a = document.activeElement;
    const b = a.getBoundingClientRect();
    return { id: a.id || a.getAttribute("aria-label") || a.tagName, top: b.top, left: b.left, value: a.value, sel: [a.selectionStart, a.selectionEnd] };
  });

  const hover = async (name, sel, tab) => {
    if (tab) await page.click(tab);
    const loc = page.locator(sel);
    if (!(await loc.count())) { r.c[name] = { error: "no element for " + sel }; return; }
    await loc.first().scrollIntoViewIfNeeded();
    const box = await loc.first().boundingBox();
    const x = box.x + box.width / 2, y = box.y + box.height / 2;
    await page.mouse.move(x, y);
    await page.evaluate(([px, py]) => { window.__hov = document.elementFromPoint(px, py); window.__hovItem = window.__hov.closest(".card, .tk"); window.__hovRect = window.__hov.getBoundingClientRect().toJSON(); window.__hovItemRect = window.__hovItem.getBoundingClientRect().toJSON(); }, [x, y]);
    let changed = 0;
    for (let i = 0; i < 20; i++) {
      if (i === 5 && name === "card option") {
        const id = await page.evaluate(() => window.__hovItem.dataset.id);
        await page.request.post(`${base}api/cards/${id}/answer`, { headers: { "X-T3CM-CSRF": csrf }, data: { text: "merge" } });
      }
      await page.waitForTimeout(1000);
      const ok = await page.evaluate(([px, py]) => {
        const el = document.elementFromPoint(px, py);
        const same = (a, b) => ["top", "left", "width", "height"].every((k) => Math.abs(a[k] - b[k]) < 1);
        if (el === window.__hov) return same(el.getBoundingClientRect(), window.__hovRect);
        const item = el && el.closest(".gone");
        return !!item && item === window.__hovItem && same(item.getBoundingClientRect(), window.__hovItemRect);
      }, [x, y]);
      if (!ok) changed++;
    }
    r.c[name] = { samplesMoved: changed };
    await page.mouse.move(700, 20);
  };
  await hover("card option", "#cards .card:not(.gone) [data-opt] >> nth=2", null);
  await hover("ticket Settle", '#done .tk:nth-child(2) [data-part="settle"]', "#tab-done");
  await page.click("#tab-open");
  const focusCases = [
    ["launch", async () => { await page.click("#issues"); await page.keyboard.type("acme/widgets#52"); }],
    ["ask", async () => { await page.click("#question"); await page.keyboard.type("what waits on me"); await page.keyboard.press("ArrowLeft"); }],
    ["card reply", async () => {
      const card = page.locator("#cards .card:not(.gone)").first();
      await card.locator('[data-part="replybtn"]').click();
      await page.keyboard.type("hold until");
    }],
    ["settled filter", async () => { await page.click("#tab-settled"); await page.click("#settled-filter"); await page.keyboard.type("acme"); }],
  ];
  for (const [name, setup] of focusCases) {
    await setup();
    const first = await probe();
    let worst = 0, lost = false;
    for (let i = 0; i < 15; i++) {
      if (name === "launch" && i === 3) r.a.answered = await answerOther();
      if (name === "ask" && i === 3) { await cut(); r.a.reconnected = true; }
      await page.waitForTimeout(1000);
      const p = await probe();
      worst = Math.max(worst, Math.abs(p.top - first.top), Math.abs(p.left - first.left));
      if (p.id !== first.id || p.value !== first.value || p.sel[0] !== first.sel[0] || p.sel[1] !== first.sel[1]) lost = true;
    }
    r.a[name] = { maxMovePx: worst, focusValueCaretKept: !lost };
    await page.keyboard.press("Escape");
    if (name === "settled filter") { await page.fill("#settled-filter", ""); await page.click("#tab-open"); }
  }

  const keep = async (name, sel, tab) => {
    if (tab) await page.click(tab);
    const ok = await page.evaluate((s) => { const el = document.querySelector(s); if (!el) return false; el.focus(); window.__kept = el; return document.activeElement === el; }, sel);
    if (!ok) { r.b[name] = { error: "no element for " + sel }; return; }
    if (name === "worker PR link") await answerOther();
    if (name === "ticket thread link") await cut();
    await page.waitForTimeout(30000);
    r.b[name] = await page.evaluate(() => ({ same: document.activeElement === window.__kept, connected: window.__kept.isConnected, active: document.activeElement.tagName }));
    await page.keyboard.press("Escape");
  };
  await keep("ticket thread link", '#open .tk:nth-child(3) [data-part="label"] a', "#tab-open");
  await keep("worker PR link", '#open .wl [data-part="wpr"] a', null);
  await keep("ask Settle", '#asks [data-part="settle"]:not([hidden])', null);
  await keep("settled Restore", '#settled .srow:nth-child(3) [data-part="restore"]', "#tab-settled");
  await page.click("#tab-open");

  return r;
}
