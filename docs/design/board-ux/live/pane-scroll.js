async (page) => {
  const base = "__BOARD_URL__";
  const copies = 6;
  const keys = new Set(["id", "thread", "ticket", "label", "handle", "link"]);
  const tag = (v, i) => {
    if (Array.isArray(v)) return v.map((x) => tag(x, i));
    if (!v || typeof v !== "object") return v;
    return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, keys.has(k) && typeof x === "string" ? `${x}-${i}` : tag(x, i)]));
  };
  const merge = (tree, i) => {
    for (const t of tree) {
      for (const w of t.workers || []) {
        if (w.pr && i) w.pr = { ...w.pr, url: `${w.pr.url}-${i}`, state: i % 2 ? "merged" : w.pr.state };
      }
    }
    return tree;
  };
  const heavy = (s) => {
    for (const k of ["cards", "tree", "feed"]) {
      s[k] = Array.from({ length: copies }, (_, i) => (k === "tree" ? merge(tag(s[k], i), i) : tag(s[k], i))).flat();
    }
    return s;
  };
  let state = null;
  await page.route("**/*", (route) => (route.request().method() === "GET" ? route.fallback() : route.abort()));
  await page.route("**/api/state", async (route) => {
    if (!state) state = heavy(await (await route.fetch()).json());
    return route.fulfill({ json: state });
  });
  await page.route("**/api/events", (route) => route.fulfill({
    contentType: "text/event-stream",
    body: `event: state\ndata: ${JSON.stringify(state)}\n\n`,
  }));

  const views = {
    dispatch: { tab: "#vt-dispatch", lists: ["#cards", "#open", "#prs", "#merged"], overflow: ["#cards", "#open"] },
    activity: { tab: "#vt-activity", lists: ["#feed", "#done"], overflow: ["#feed"] },
  };
  const sizes = [[1600, 900], [1121, 600], [1100, 800], [1100, 700], [641, 700]];
  const pageSizes = [[1121, 599], [1100, 699], [641, 540], [390, 844]];
  const usable = 60;
  const report = { base, pass: true, runs: [] };
  const read = (lists) => page.evaluate((lists) => {
    const el = (s) => document.querySelector(s);
    const box = (e) => ({ sh: e.scrollHeight, ch: e.clientHeight, st: Math.round(e.scrollTop) });
    return { doc: box(document.scrollingElement), main: box(el("#main")), lists: Object.fromEntries(lists.map((s) => [s, box(el(s))])) };
  }, lists);

  for (const [w, h] of sizes) {
    await page.setViewportSize({ width: w, height: h });
    await page.goto(base);
    await page.waitForFunction(() => document.querySelectorAll("#cards > *").length > 10 && document.querySelectorAll("#open > *").length > 10);
    for (const [name, v] of Object.entries(views)) {
      await page.click(v.tab);
      await page.waitForTimeout(300);
      const run = { size: `${w}x${h}`, view: name, fails: [], wheel: {} };
      const at = await read(v.lists);
      run.layout = at;
      if (at.doc.sh > at.doc.ch) run.fails.push(`document overflows: ${at.doc.sh} > ${at.doc.ch}`);
      if (at.main.sh > at.main.ch + 1) run.fails.push(`#main overflows: ${at.main.sh} > ${at.main.ch}`);
      for (const s of v.lists) {
        if (at.lists[s].ch < usable) run.fails.push(`${s} is ${at.lists[s].ch}px tall`);
      }
      for (const s of v.overflow) {
        if (at.lists[s].sh <= at.lists[s].ch) run.fails.push(`${s} does not overflow its pane`);
      }
      for (const s of v.lists) {
        await page.evaluate(() => { for (const e of document.querySelectorAll("*")) e.scrollTop = 0; });
        const b = await page.locator(s).boundingBox();
        if (!b || b.y + 20 > h) { run.fails.push(`${s} is off screen`); continue; }
        await page.mouse.move(b.x + b.width / 2, b.y + Math.min(b.height, h - b.y) / 2);
        await page.mouse.wheel(0, 300);
        await page.waitForTimeout(400);
        const after = await read(v.lists);
        run.wheel[s] = { list: after.lists[s].st, main: after.main.st, doc: after.doc.st };
        if (after.doc.st || after.main.st) run.fails.push(`wheel over ${s} scrolled the page`);
        const scrolls = at.lists[s].sh > at.lists[s].ch;
        if (scrolls && !after.lists[s].st) run.fails.push(`wheel over ${s} did not scroll it`);
      }
      if (run.fails.length) report.pass = false;
      report.runs.push(run);
    }
  }
  for (const [w, h] of pageSizes) {
    await page.setViewportSize({ width: w, height: h });
    await page.goto(base);
    await page.waitForFunction(() => document.querySelectorAll("#cards > *").length > 10);
    for (const [name, v] of Object.entries(views)) {
      await page.click(v.tab);
      await page.waitForTimeout(300);
      const s = v.overflow[0];
      const run = { size: `${w}x${h}`, view: `${name} page scroll`, fails: [], wheel: {} };
      await page.evaluate(() => { for (const e of document.querySelectorAll("*")) e.scrollTop = 0; });
      const b = await page.locator(s).boundingBox();
      await page.mouse.move(b.x + b.width / 2, Math.min(b.y + 30, h - 10));
      await page.mouse.wheel(0, 300);
      await page.waitForTimeout(400);
      const after = await read([s]);
      run.wheel[s] = { list: after.lists[s].st, main: after.main.st, doc: after.doc.st };
      if (!after.main.st) run.fails.push(`wheel over ${s} did not scroll #main`);
      if (after.doc.st) run.fails.push(`wheel over ${s} scrolled the document`);
      if (run.fails.length) report.pass = false;
      report.runs.push(run);
    }
  }
  report.summary = report.runs.map((r) => `${r.size} ${r.view}: ${r.fails.length ? r.fails.join("; ") : "ok"}`).join(" | ");
  return report;
}
