async (page) => {
  const base = "__BOARD_URL__";
  const r = {};
  await page.addInitScript(() => {
    const ES = window.EventSource; window.__es = [];
    window.EventSource = function (...a) { const e = new ES(...a); window.__es.push(e); e.addEventListener("state", () => { window.__states = (window.__states || 0) + 1; }); return e; };
    window.EventSource.prototype = ES.prototype;
    window.__said = [];
    window.__notes = [];
    const RN = window.Notification;
    if (RN) {
      const W = function (t, o) {
        const note = new RN(t, o);
        let pending = true;
        const timer = setTimeout(() => { pending = false; }, 10000);
        note.addEventListener("show", () => {
          if (!pending) return;
          pending = false;
          clearTimeout(timer);
          window.__notes.push(t);
        }, { once: true });
        note.addEventListener("error", () => { pending = false; clearTimeout(timer); }, { once: true });
        return note;
      };
      Object.defineProperty(W, "permission", { get: () => RN.permission });
      W.requestPermission = (...a) => RN.requestPermission(...a);
      window.Notification = W;
    }
    document.addEventListener("DOMContentLoaded", () => {
      for (const id of ["live", "alert"]) {
        const el = document.getElementById(id);
        new MutationObserver(() => { if (el.textContent) window.__said.push([id, el.textContent]); }).observe(el, { childList: true, characterData: true, subtree: true });
      }
    });
  });
  await page.emulateMedia({ colorScheme: "light", reducedMotion: "reduce" });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(base);
  await page.waitForSelector("#cards .card");
  await page.waitForTimeout(1000);

  const firstVisible = (id) => page.evaluate((lid) => {
    const list = document.getElementById(lid);
    const top = list.getBoundingClientRect().top;
    const el = [...list.children].find((c) => c.getBoundingClientRect().bottom > top + 1);
    return el ? { key: el.dataset.key, offset: Math.round(el.getBoundingClientRect().top - top) } : null;
  }, id);
  await page.mouse.move(700, 20);
  await page.evaluate(() => {
    for (const id of ["cards", "open", "feed"]) { const el = document.getElementById(id); el.scrollTop = Math.max(40, (el.scrollHeight - el.clientHeight) / 2); }
  });
  await page.waitForTimeout(500);
  const before = { cards: await firstVisible("cards"), open: await firstVisible("open"), feed: await firstVisible("feed") };
  await page.waitForTimeout(60000);
  const after = { cards: await firstVisible("cards"), open: await firstVisible("open"), feed: await firstVisible("feed") };
  r.U4 = { before, after, same: JSON.stringify(before) === JSON.stringify(after),
    slots: await page.evaluate(() => ({ cards: document.getElementById("new-cards").textContent, tickets: document.getElementById("new-tix").textContent, feed: document.getElementById("new-feed").textContent })) };
  const openTop = await page.evaluate(() => document.getElementById("open").scrollTop);
  await page.click("#tab-done");
  await page.click("#tab-settled");
  await page.click("#tab-open");
  r.U4.tabRestore = { before: openTop, after: await page.evaluate(() => document.getElementById("open").scrollTop) };
  if (await page.locator("#new-cards:not(.off)").count()) await page.click("#new-cards");
  r.U4.afterApply = await page.evaluate(() => ({ slot: document.getElementById("new-cards").className, top: document.getElementById("cards").scrollTop, focus: document.activeElement.dataset.id }));

  const rects = () => page.evaluate(() => Object.fromEntries(["p-dec", "p-tix", "p-act", "composer", "issues", "question"].map((id) => {
    const b = document.getElementById(id).getBoundingClientRect(); return [id, [b.top, b.left, b.width, b.height].map(Math.round).join()];
  })));
  const r0 = await rects();
  await page.route("**/api/events", (route) => route.abort());
  await page.evaluate(() => window.__es.forEach((e) => { e.close(); e.dispatchEvent(new Event("error")); }));
  await page.waitForTimeout(2000);
  r.U11 = { during: await page.evaluate(() => ({
    header: document.getElementById("svc-text").textContent,
    stale: document.body.classList.contains("stale"),
    optionsDisabled: [...document.querySelectorAll("#cards [data-opt], #cards [data-part=dismiss], #cards [data-part=send]")].every((b) => b.getAttribute("aria-disabled") === "true"),
  })), rectsSame: JSON.stringify(r0) === JSON.stringify(await rects()) };
  await page.waitForTimeout(3000);
  const s = await page.evaluate(() => window.__states || 0);
  await page.unroute("**/api/events");
  await page.waitForFunction((n) => (window.__states || 0) > n, s, { timeout: 40000 });
  r.U11.after = await page.evaluate(() => ({ header: document.getElementById("svc-text").textContent, stale: document.body.classList.contains("stale") }));
  r.U11.rectsSameAfter = JSON.stringify(r0) === JSON.stringify(await rects());

  await page.evaluate(() => { window.__said = []; });
  await page.route("**/api/ask", (route) => route.fulfill({ status: 502, contentType: "application/json", body: JSON.stringify({ error: "T3 is not answering" }) }));
  await page.fill("#question", "what waits on me");
  await page.click("#ask-btn");
  await page.waitForTimeout(800);
  await page.unroute("**/api/ask");
  await page.fill("#issues", "acme/widgets#77\nacme/nowhere#3");
  await page.click("#launch-btn");
  await page.waitForTimeout(1500);
  await page.route("**/api/cards/*/answer", (route) => route.fulfill({ status: 400, contentType: "application/json", body: JSON.stringify({ error: "the ticket thread is gone" }) }));
  const opt = page.locator("#cards .card:not(.gone) [data-opt]").first();
  await opt.click();
  await page.waitForTimeout(6000);
  await page.unroute("**/api/cards/*/answer");
  await page.locator("#cards .card").nth(1).hover();
  await page.waitForTimeout(8000);
  const said = await page.evaluate(() => window.__said);
  const count = (re) => said.filter(([, t]) => re.test(t)).length;
  r.F3 = {
    said,
    askError: { n: count(/^Ask failed/), alert: said.some(([id, t]) => id === "alert" && /^Ask failed/.test(t)) },
    launch: { ok: count(/^Launched widgets#77/), failed: count(/nowhere#3/), shown: await page.evaluate(() => ({ good: document.querySelectorAll("#launch-out .good").length, bad: document.querySelectorAll("#launch-out .bad").length })) },
    cardError: { n: count(/the ticket thread is gone/), alert: said.some(([id, t]) => id === "alert" && /thread is gone/.test(t)) },
    newDecisions: { n: count(/new decision/) },
    activity: count(/note:|sol-loop|cargo/),
  };
  await page.context().grantPermissions(["notifications"]);
  await page.locator("#notify").click().catch(() => {});
  const notify = await page.evaluate(() => document.getElementById("notify").textContent);
  const cardsBefore = await page.evaluate(() => (window.__notes = [], document.querySelectorAll("#cards .card:not(.gone)").length));
  await page.evaluate(() => {
    Object.defineProperty(document, "hidden", { configurable: true, get: () => true });
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "hidden" });
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await page.waitForFunction(() => window.__notes.length > 0, null, { timeout: 60000 }).catch(() => {});
  r.F1 = { notification: { button: notify, cardsBefore, notes: await page.evaluate(() => window.__notes) } };
  return r;
}
