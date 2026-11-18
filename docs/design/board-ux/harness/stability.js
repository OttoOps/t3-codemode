async (page) => {
  const base = "__BOARD_URL__";
  const r = { a: {}, b: {}, c: {} };
  await page.addInitScript(() => {
    const ES = window.EventSource; window.__es = [];
    window.EventSource = function (...a) { const e = new ES(...a); window.__es.push(e); e.addEventListener("state", () => { window.__states = (window.__states || 0) + 1; }); return e; };
    window.EventSource.prototype = ES.prototype;
    window.__obs = null;
    setInterval(() => { const o = window.__obs; if (o) { o.n++; o.tick(o); } }, 20);
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
  const stop = () => page.evaluate(() => { const o = window.__obs; window.__obs = null; delete o.tick; return o; });

  const hover = async (name, sel, tab) => {
    if (tab) await page.click(tab);
    const loc = page.locator(sel);
    if (!(await loc.count())) { r.c[name] = { error: "no element for " + sel }; return; }
    await loc.first().scrollIntoViewIfNeeded();
    await loc.first().evaluate((el) => { window.__target = el; });
    const box = await loc.first().boundingBox();
    const x = box.x + box.width / 2, y = box.y + box.height / 2;
    await page.mouse.move(x, y);
    const hit = await page.evaluate(([px, py]) => {
      const hov = document.elementFromPoint(px, py);
      const item = hov && window.__target.contains(hov) && hov.closest(".card, .tk");
      if (!item) return false;
      const same = (a, b) => ["top", "left", "width", "height"].every((k) => Math.abs(a[k] - b[k]) < 1);
      const hb = hov.getBoundingClientRect(), ib = item.getBoundingClientRect();
      window.__hovItem = item;
      window.__obs = { n: 0, moved: 0, tick(o) {
        const el = document.elementFromPoint(px, py);
        if (el === hov) { if (!same(el.getBoundingClientRect(), hb)) o.moved++; return; }
        const g = el && el.closest(".gone");
        if (!(g && g === item && same(g.getBoundingClientRect(), ib))) o.moved++;
      } };
      return true;
    }, [x, y]);
    if (!hit) { r.c[name] = { error: "hover did not land on " + sel }; await page.mouse.move(700, 20); return; }
    const out = {};
    for (let i = 0; i < 60; i++) {
      if (i === 1 && name === "card option") {
        const id = await page.evaluate(() => window.__hovItem.dataset.id);
        const res = await page.request.post(`${base}api/cards/${id}/answer`, { headers: { "X-T3CM-CSRF": csrf }, data: { text: "merge" } });
        out.answer = { id, status: res.status(), error: res.ok() ? null : (await res.json().catch(() => ({}))).error || null };
      }
      if (i === 10 && name === "ticket Settle") { await cut(); out.reconnected = true; }
      await page.waitForTimeout(1000);
    }
    if (name === "card option") out.removed = await page.evaluate(() => window.__hovItem.classList.contains("gone") || !window.__hovItem.isConnected);
    const o = await stop();
    r.c[name] = { ...out, samples: o.n, samplesMoved: o.moved };
    await page.mouse.move(700, 20);
  };
  const focusCases = [
    ["launch", async () => { await page.click("#issues"); await page.keyboard.type("acme/widgets#52"); }],
    ["ask", async () => { await page.click("#question"); await page.keyboard.type("what waits on me"); await page.keyboard.press("ArrowLeft"); }],
    ["card reply", async () => {
      const card = page.locator("#cards .card:not(.gone)").filter({ has: page.locator('[data-part="replybtn"]:not([hidden]):not([aria-disabled="true"]):enabled') }).first();
      await card.locator('[data-part="replybtn"]').click();
      await page.keyboard.type("hold until");
    }],
    ["settled filter", async () => { await page.click("#tab-settled"); await page.click("#settled-filter"); await page.keyboard.type("acme"); }],
  ];
  for (const [name, setup] of focusCases) {
    await setup();
    await page.evaluate(() => {
      const a = document.activeElement;
      const b0 = a.getBoundingClientRect();
      const v = a.value, s0 = a.selectionStart, s1 = a.selectionEnd;
      window.__obs = { n: 0, worst: 0, lost: false, tick(o) {
        if (document.activeElement !== a) { o.lost = true; return; }
        const b = a.getBoundingClientRect();
        o.worst = Math.max(o.worst, Math.abs(b.top - b0.top), Math.abs(b.left - b0.left));
        if (a.value !== v || a.selectionStart !== s0 || a.selectionEnd !== s1) o.lost = true;
      } };
    });
    for (let i = 0; i < 15; i++) {
      if (name === "launch" && i === 3) r.a.answered = await answerOther();
      if (name === "ask" && i === 3) { await cut(); r.a.reconnected = true; }
      await page.waitForTimeout(1000);
    }
    const o = await stop();
    r.a[name] = { samples: o.n, maxMovePx: o.worst, focusValueCaretKept: !o.lost };
    await page.keyboard.press("Escape");
    if (name === "settled filter") { await page.fill("#settled-filter", ""); await page.click("#tab-open"); }
  }

  await hover("card option", '#cards .card:not(.gone):has([data-part="replybtn"]:not([hidden])):has([data-opt]) >> nth=-1 >> [data-opt] >> nth=0', null);
  await hover("ticket Settle", '#done .tk:nth-child(2) [data-part="settle"]', "#tab-done");
  await page.click("#tab-open");

  const keep = async (name, sel, tab) => {
    if (tab) await page.click(tab);
    const ok = await page.evaluate((s) => {
      const el = document.querySelector(s);
      if (!el) return false;
      el.focus();
      if (document.activeElement !== el) return false;
      window.__kept = el;
      window.__obs = { n: 0, lost: 0, seen: [], tick(o) {
        const a = document.activeElement;
        if (a === el && el.isConnected) return;
        o.lost++;
        if (o.seen.length < 5 && !o.seen.includes(a.tagName)) o.seen.push(a.tagName);
      } };
      return true;
    }, sel);
    if (!ok) { r.b[name] = { error: "no element for " + sel }; return; }
    if (name === "worker PR link") await answerOther();
    if (name === "ticket thread link") await cut();
    await page.waitForTimeout(30000);
    const o = await stop();
    r.b[name] = { ...(await page.evaluate(() => ({ same: document.activeElement === window.__kept, connected: window.__kept.isConnected, active: document.activeElement.tagName }))), samples: o.n, samplesLost: o.lost, seen: o.seen };
    await page.keyboard.press("Escape");
  };
  await keep("ticket thread link", '#open .tk:nth-child(3) [data-part="label"] a', "#tab-open");
  await keep("worker PR link", '#open .wl [data-part="wpr"] a', null);
  await keep("ask Settle", '#asks [data-part="settle"]:not([hidden])', null);
  await keep("settled Restore", '#settled .srow:nth-child(3) [data-part="restore"]', "#tab-settled");
  await page.click("#tab-open");

  return r;
}
