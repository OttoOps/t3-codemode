async (page) => {
  await page.addInitScript(() => {
    const ES = window.EventSource; window.__es = [];
    window.EventSource = function (...a) { const e = new ES(...a); window.__es.push(e); e.addEventListener("state", () => { window.__states = (window.__states || 0) + 1; }); return e; };
    window.EventSource.prototype = ES.prototype;
  });
  await page.goto("__BOARD_URL__");
  await page.waitForSelector(".card");
  await page.locator("#issues").click();
  await page.keyboard.type("acme/widgets#52");
  const top0 = await page.evaluate(() => Math.round(document.activeElement.getBoundingClientRect().top));
  await page.route("**/api/events", (r) => r.abort());
  await page.evaluate(() => window.__es.forEach((e) => { e.close(); e.dispatchEvent(new Event("error")); }));
  await page.waitForTimeout(1500);
  const banner = await page.evaluate(() => document.getElementById("banner").classList.contains("on"));
  const s1 = await page.evaluate(() => window.__states || 0);
  await page.waitForTimeout(3500);
  const s2 = await page.evaluate(() => window.__states || 0);
  await page.unroute("**/api/events");
  await page.waitForFunction((n) => (window.__states || 0) > n, s2, { timeout: 35000 });
  const after = await page.evaluate(() => ({ banner: document.getElementById("banner").classList.contains("on"), focus: document.activeElement.id, top: Math.round(document.activeElement.getBoundingClientRect().top), value: document.activeElement.value }));
  return { top0, bannerWhileCut: banner, statesWhileCut: [s1, s2], after };
}
