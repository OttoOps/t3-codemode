async (page) => {
  const base = "__BOARD_URL__";
  const out = "__CAPTURE_DIR__";
  const tag = "__TAG__";
  await page.route("**/*", (route) => (route.request().method() === "GET" ? route.continue() : route.abort()));
  await page.clock.setFixedTime(new Date("2026-11-19T00:26:30Z"));
  const views = [
    { name: "1920-light", w: 1920, h: 1080, scheme: "light" },
    { name: "1366-light", w: 1366, h: 768, scheme: "light" },
    { name: "1366-dark", w: 1366, h: 768, scheme: "dark" },
    { name: "1024-light", w: 1024, h: 768, scheme: "light" },
    { name: "390-light-tickets", w: 390, h: 844, scheme: "light", click: '#mtabs [data-panel="tickets"]' },
    { name: "1366-light-dismissed", w: 1366, h: 768, scheme: "light", click: "#new-view" },
  ];
  const report = {};
  for (const v of views) {
    await page.setViewportSize({ width: v.w, height: v.h });
    await page.emulateMedia({ colorScheme: v.scheme });
    await page.goto(base);
    await page.waitForTimeout(800);
    if (v.click && await page.locator(v.click).isVisible()) await page.click(v.click);
    await page.waitForTimeout(200);
    await page.mouse.move(0, 0);
    await page.screenshot({ path: `${out}/${tag}-${v.name}.png` });
    report[v.name] = await page.evaluate(() => {
      const r = (id) => { const el = document.getElementById(id); if (!el) return null; const b = el.getBoundingClientRect(); return [Math.round(b.top), Math.round(b.height)]; };
      return { pageScroll: document.documentElement.scrollHeight > innerHeight + 1, newSection: r("p-new"), tickets: r("p-tix"), sideways: document.documentElement.scrollWidth > innerWidth };
    });
  }
  return report;
}
