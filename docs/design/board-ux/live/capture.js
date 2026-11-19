async (page) => {
  const base = "__BOARD_URL__";
  const out = "__CAPTURE_DIR__";
  const now = "__NOW__";
  const report = { base, now, views: {} };

  await page.route("**/*", (route) => {
    if (route.request().method() !== "GET") return route.abort();
    return route.continue();
  });
  if (now) await page.clock.setFixedTime(new Date(now));

  const measure = () => page.evaluate(() => {
    const vis = (el) => {
      const r = el.getBoundingClientRect();
      if (!r.width || !r.height) return false;
      let x0 = Math.max(r.left, 0), y0 = Math.max(r.top, 0);
      let x1 = Math.min(r.right, innerWidth), y1 = Math.min(r.bottom, innerHeight);
      for (let a = el.parentElement; a && a !== document.body; a = a.parentElement) {
        const cs = getComputedStyle(a);
        if (cs.overflowX === "visible" && cs.overflowY === "visible") continue;
        const c = a.getBoundingClientRect();
        x0 = Math.max(x0, c.left); y0 = Math.max(y0, c.top);
        x1 = Math.min(x1, c.right); y1 = Math.min(y1, c.bottom);
      }
      if (x1 <= x0 || y1 <= y0) return false;
      return el.closest("[hidden]") === null && getComputedStyle(el).visibility !== "hidden";
    };
    const clipped = [];
    for (const el of document.querySelectorAll("body *")) {
      if (!vis(el) || !el.textContent.trim()) continue;
      const cs = getComputedStyle(el);
      const r0 = el.getBoundingClientRect();
      if (r0.width <= 1 || r0.height <= 1 || /rect\(0px/.test(cs.clip) || /inset\(50%\)/.test(cs.clipPath)) continue;
      if (/auto|scroll/.test(cs.overflowY) && el.scrollHeight > el.clientHeight + 1) continue;
      const clamp = cs.webkitLineClamp && cs.webkitLineClamp !== "none" && el.scrollHeight > el.clientHeight + 1;
      const over = clamp || (el.scrollWidth > el.clientWidth + 1 && cs.overflowX !== "visible");
      const parent = el.parentElement && el.parentElement.getBoundingClientRect();
      const r = el.getBoundingClientRect();
      const cut = parent && el.parentElement.scrollWidth > el.parentElement.clientWidth + 1 &&
        getComputedStyle(el.parentElement).overflowX !== "visible" && r.right > parent.right + 1;
      if (over || cut) clipped.push({
        text: el.textContent.trim().replace(/\s+/g, " ").slice(0, 80),
        cls: el.className && String(el.className).slice(0, 40),
        ellipsis: cs.textOverflow === "ellipsis" || !!clamp,
        hasTitle: !!(el.title || (el.closest("[title]") && el.closest("[title]").title)),
        how: over ? "overflow" : "cut-by-parent",
      });
    }
    const sizes = new Set(), weights = new Set();
    for (const el of document.querySelectorAll("body *")) {
      if (!vis(el)) continue;
      const own = [...el.childNodes].some((n) => n.nodeType === 3 && n.textContent.trim());
      if (!own) continue;
      const cs = getComputedStyle(el);
      if (/mono/.test(cs.fontFamily)) continue;
      sizes.add(cs.fontSize); weights.add(cs.fontWeight);
    }
    const panel = (sel) => {
      const el = document.querySelector(sel);
      if (!el) return null;
      const r = el.getBoundingClientRect();
      return { top: Math.round(r.top), left: Math.round(r.left), width: Math.round(r.width), height: Math.round(r.height), scrollHeight: el.scrollHeight };
    };
    return {
      viewport: [innerWidth, innerHeight],
      scrollHeight: document.scrollingElement.scrollHeight,
      clipped,
      clippedCount: clipped.length,
      clippedNoTitle: clipped.filter((c) => !c.hasTitle).length,
      fontSizes: [...sizes], fontWeights: [...weights],
      cards: document.querySelectorAll(".card").length,
      panels: { cards: panel("#cards"), open: panel("#open"), feed: panel("#feed") },
    };
  });

  const views = [
    { name: "1920-dark", w: 1920, h: 1080, scheme: "dark" },
    { name: "1920-light", w: 1920, h: 1080, scheme: "light" },
    { name: "1366-dark", w: 1366, h: 768, scheme: "dark" },
    { name: "390-dark", w: 390, h: 844, scheme: "dark" },
  ];
  for (const v of views) {
    await page.emulateMedia({ colorScheme: v.scheme, reducedMotion: "reduce" });
    await page.setViewportSize({ width: v.w, height: v.h });
    await page.goto(base);
    await page.waitForSelector(".card");
    await page.waitForTimeout(1500);
    await page.screenshot({ path: `${out}/${v.name}.png` });
    report.views[v.name] = await measure();
    if (v.w < 700) {
      const tabs = await page.evaluate(() => [...document.querySelectorAll("[role=tab]")]
        .filter((t) => t.getBoundingClientRect().width > 0).map((t) => t.id || t.textContent.trim()));
      report.views[v.name].tabs = tabs;
      for (const id of tabs) {
        const ok = await page.evaluate((id) => {
          const t = document.getElementById(id) || [...document.querySelectorAll("[role=tab]")].find((x) => x.textContent.trim() === id);
          if (!t || t.closest("#tix-tabs")) return false;
          t.click();
          return true;
        }, id);
        if (!ok) continue;
        await page.waitForTimeout(500);
        const slug = String(id).toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
        await page.screenshot({ path: `${out}/${v.name}-${slug}.png` });
        report.views[`${v.name}-${slug}`] = await measure();
      }
    }
  }
  return JSON.stringify(report, null, 1);
}
