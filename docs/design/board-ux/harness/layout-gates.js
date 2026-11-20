async (page) => {
  const base = "__BOARD_URL__";
  const mainUrl = "__MAIN_URL__";
  const diffDir = "__CAPTURE_DIR__";
  const out = {};
  const errs = [];
  page.on("pageerror", (e) => errs.push(String(e)));
  const KEYS = { dec: "p-dec", new: "p-new", tix: "p-tix", cmd: "composer", act: "p-act" };
  const DEFAULT = [["p-dec"], ["p-new", "p-tix"], ["composer", "p-act"]];
  const where = () => page.evaluate(() => [...document.querySelectorAll(".lcol")].map((c) => [...c.children].filter((x) => x.tagName === "SECTION").map((x) => x.id)));
  const fresh = async (w = 1440, h = 900) => {
    await page.setViewportSize({ width: w, height: h });
    await page.goto(base);
    await page.evaluate(() => localStorage.clear());
    await page.reload();
    if (w >= 1200) await page.waitForSelector(".lcol", { state: "attached" });
    await page.waitForTimeout(300);
  };
  const head = (id) => page.locator(id === "composer" ? "#launch .ph" : "#" + id + " > .ph");
  const box = async (sel) => page.locator(sel).first().boundingBox();
  const dragTo = async (id, x, y) => {
    const b = await head(id).boundingBox();
    await page.mouse.move(b.x + 8 + 40, b.y + b.height / 2);
    await page.mouse.down();
    await page.mouse.move(b.x + 60, b.y + b.height / 2 + 8, { steps: 3 });
    await page.mouse.move(x, y, { steps: 8 });
    await page.mouse.up();
    await page.waitForTimeout(250);
  };
  const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
  const geometry = () => page.evaluate(() => [...document.querySelectorAll("body *")]
    .filter((el) => el.id !== "lay-hint" && el.getClientRects().length)
    .map((el) => { const r = el.getBoundingClientRect(); return [el.tagName, el.id, el.className, Math.round(r.left), Math.round(r.top), Math.round(r.width), Math.round(r.height)].join(" "); }));

  const page1 = () => page.evaluate(() => {
    const se = document.scrollingElement;
    return se.scrollHeight <= innerHeight && se.scrollWidth <= innerWidth;
  });
  const clipped = () => page.evaluate(() => {
    const bad = [];
    for (const b of document.querySelectorAll(".lctl button, .ph h2")) {
      const col = b.closest(".lcol");
      if (!col || col.classList.contains("vacant")) continue;
      const r = b.getBoundingClientRect(), c = col.getBoundingClientRect();
      if (r.width && (r.left < c.left - 0.5 || r.right > c.right + 0.5)) bad.push(b.closest("section, #composer").id + " " + (b.className || b.tagName));
    }
    return bad;
  });

  await fresh();
  const l1 = { pointer: [], keyboard: [] };
  for (const [k, id] of Object.entries(KEYS)) {
    for (let ci = 0; ci < 3; ci++) {
      for (const pos of ["top", "bottom"]) {
        await fresh();
        const cols = await page.locator(".lcol").all();
        let r = await cols[ci].boundingBox();
        if (!r || r.width < 8) {
          await page.mouse.move(0, 0);
          const b = await head(id).boundingBox();
          await page.mouse.move(b.x + 48, b.y + b.height / 2);
          await page.mouse.down();
          await page.mouse.move(b.x + 64, b.y + b.height / 2 + 8, { steps: 3 });
          r = await cols[ci].boundingBox();
          await page.mouse.move(r.x + r.width / 2, r.y + r.height / 2, { steps: 8 });
          await page.mouse.up();
          await page.waitForTimeout(250);
        } else {
          await dragTo(id, r.x + r.width / 2, pos === "top" ? r.y + 6 : r.y + r.height - 6);
        }
        const w = await where();
        const col = w[ci];
        const ok = pos === "top" ? col[0] === id : col[col.length - 1] === id;
        l1.pointer.push({ id, ci, pos, ok });
      }
    }
  }
  for (const [k, id] of Object.entries(KEYS)) {
    await fresh();
    const reach = new Set();
    const g = page.locator((id === "composer" ? "#launch" : "#" + id) + " .grip");
    for (const seq of [["ArrowLeft", "ArrowLeft"], ["ArrowRight", "ArrowRight"], ["ArrowUp", "ArrowUp", "ArrowUp"], ["ArrowDown", "ArrowDown", "ArrowDown"]]) {
      for (const key of seq) {
        await g.focus();
        await page.keyboard.press(key);
        const w = await where();
        w.forEach((c, i) => { const j = c.indexOf(id); if (j >= 0) reach.add(i + ":" + (j === 0 ? "top" : j === c.length - 1 ? "bottom" : "mid")); });
      }
    }
    const focusKept = await page.evaluate((id) => document.activeElement && document.activeElement.classList.contains("grip") && document.activeElement.closest("#" + id) !== null, id);
    l1.keyboard.push({ id, reach: [...reach].sort(), focusKept });
  }
  l1.pass = l1.pointer.every((x) => x.ok) && l1.keyboard.every((x) => x.focusKept && [0, 1, 2].every((i) => [...x.reach].some((r) => r.startsWith(i + ":"))));
  out.L1 = l1;

  await fresh();
  const l2 = {};
  await page.locator("#p-new .fold").click();
  l2.chevron = await page.evaluate(() => [document.getElementById("p-new").classList.contains("collapsed"), document.querySelector("#p-new .fold").getAttribute("aria-expanded")]);
  await page.locator("#p-new .fold").press("Enter");
  l2.enter = await page.evaluate(() => document.getElementById("p-new").classList.contains("collapsed"));
  const hb = await head("p-act").boundingBox();
  await page.mouse.dblclick(hb.x + hb.width / 2, hb.y + hb.height / 2);
  await page.waitForTimeout(200);
  l2.dblclick = await page.evaluate(() => document.getElementById("p-act").classList.contains("collapsed"));
  await page.locator("body").click({ position: { x: 5, y: 5 } });
  await page.keyboard.press("g");
  await page.keyboard.press("a");
  await page.waitForTimeout(200);
  l2.shortcutExpands = await page.evaluate(() => !document.getElementById("p-act").classList.contains("collapsed"));
  l2.pass = same(l2.chevron, [true, "false"]) && l2.enter === false && l2.dblclick === true && l2.shortcutExpands;
  out.L2 = l2;

  const l3 = { layouts: [] };
  await fresh();
  const v = await box(".vsplit");
  const w0 = (await box(".lcol[data-i='0']")).width;
  await page.mouse.move(v.x + 0.5, v.y + 300);
  await page.mouse.down();
  await page.mouse.move(v.x + 100.5, v.y + 300, { steps: 6 });
  await page.mouse.up();
  const w1 = (await box(".lcol[data-i='0']")).width;
  l3.pointerCol = Math.round(w1 - w0);
  await page.locator(".vsplit").first().focus();
  await page.keyboard.press("ArrowLeft");
  l3.keyCol = Math.round((await box(".lcol[data-i='0']")).width - w1);
  const hs = await box(".lcol[data-i='1'] > .hsplit:not(.fixed)");
  const n0 = (await box("#p-new")).height;
  await page.mouse.move(hs.x + 200, hs.y + 0.5);
  await page.mouse.down();
  await page.mouse.move(hs.x + 200, hs.y + 60.5, { steps: 6 });
  await page.mouse.up();
  l3.pointerRow = Math.round((await box("#p-new")).height - n0);
  await page.locator(".lcol[data-i='1'] > .hsplit:not(.fixed)").focus();
  const n1 = (await box("#p-new")).height;
  await page.keyboard.press("ArrowDown");
  l3.keyRow = Math.round((await box("#p-new")).height - n1);
  for (const [w, h] of [[1440, 900], [1280, 800], [1200, 700], [1920, 1080]]) {
    for (const squeeze of [-5000, 5000]) {
      await fresh(w, h);
      for (const i of [0, 1]) {
        const s = await page.locator(".vsplit").nth(i).boundingBox();
        await page.mouse.move(s.x + 0.5, s.y + 200);
        await page.mouse.down();
        await page.mouse.move(s.x + squeeze, s.y + 200, { steps: 4 });
        await page.mouse.up();
      }
      const widths = await page.evaluate(() => [...document.querySelectorAll(".lcol:not(.vacant)")].map((c) => Math.round(c.getBoundingClientRect().width)));
      l3.layouts.push({ w, h, squeeze, widths, noScroll: await page1(), clipped: await clipped() });
    }
    await fresh(w, h);
    await page.evaluate(() => {
      localStorage.setItem("t3board.layout", JSON.stringify({ cols: [{ w: null, p: [] }, { w: null, p: ["dec", "new", "tix", "act", "cmd"] }, { w: null, p: [] }], h: {}, c: {} }));
    });
    await page.reload();
    await page.waitForTimeout(400);
    l3.layouts.push({ w, h, squeeze: "one column", widths: [], noScroll: await page1(), clipped: await clipped() });
  }
  l3.pass = l3.pointerCol === 100 && l3.keyCol === -24 && l3.pointerRow === 60 && l3.keyRow === 24 &&
    l3.layouts.every((x) => x.noScroll && !x.clipped.length && x.widths.every((n) => n >= 320));
  out.L3 = l3;

  await fresh();
  const l4 = {};
  await dragTo("p-dec", (await box(".lcol[data-i='1']")).x + 200, (await box("#p-tix")).y + 4);
  await page.locator("#p-act .fold").click();
  const before = await where();
  await page.reload();
  await page.waitForTimeout(400);
  l4.reload = same(await where(), before) && await page.evaluate(() => document.getElementById("p-act").classList.contains("collapsed"));
  l4.resetShown = await page.locator("#lay-reset").isVisible();
  await page.locator("#lay-reset").click();
  await page.waitForTimeout(300);
  l4.reset = same(await where(), DEFAULT) && !(await page.locator("#lay-reset").isVisible()) && await page.evaluate(() => localStorage.getItem("t3board.layout") === null);
  const bad = [];
  for (const raw of ["{", "[]", JSON.stringify({ cols: [{ p: ["dec"] }, { p: ["new"] }, { p: ["tix", "cmd", "act", "act"] }] }), JSON.stringify({ cols: [{ w: -4, p: ["dec"] }, { p: ["new", "tix"] }, { p: ["cmd", "act"] }] })]) {
    await page.evaluate((raw) => localStorage.setItem("t3board.layout", raw), raw);
    await page.reload();
    await page.waitForTimeout(300);
    bad.push(same(await where(), DEFAULT));
  }
  l4.fallback = bad;
  l4.pass = l4.reload && l4.resetShown && l4.reset && bad.every(Boolean);
  out.L4 = l4;

  const l5 = { views: [] };
  const custom = JSON.stringify({ cols: [{ w: 300, p: ["act"] }, { w: 900, p: ["dec", "new"] }, { w: 400, p: ["tix", "cmd"] }], h: { dec: 400, new: 100 }, c: { new: true } });
  for (const [w, h] of [[1024, 768], [768, 1024], [390, 844]]) {
    for (const scheme of ["light", "dark"]) {
      await page.emulateMedia({ colorScheme: scheme, reducedMotion: "reduce" });
      await page.setViewportSize({ width: w, height: h });
      await page.mouse.move(0, 0);
      await page.goto(mainUrl);
      await page.waitForTimeout(600);
      const gMain = await geometry();
      const before = await page.screenshot({ path: `${diffDir}/narrow-${w}-${scheme}-main.png` });
      await fresh(w, h);
      await page.waitForTimeout(300);
      const gPlain = await geometry();
      const plain = await page.screenshot({ path: `${diffDir}/narrow-${w}-${scheme}-plain.png` });
      await page.evaluate((c) => localStorage.setItem("t3board.layout", c), custom);
      await page.reload();
      await page.waitForTimeout(400);
      const gSaved = await geometry();
      const saved = await page.screenshot({ path: `${diffDir}/narrow-${w}-${scheme}-saved.png` });
      const diff = (a, b) => a.map((x, i) => (x === b[i] ? null : [x, b[i]])).filter(Boolean).slice(0, 4);
      l5.views.push({ w, h, scheme, sameAsMain: same(gMain, gPlain), identical: same(gPlain, gSaved), elements: gMain.length, mainDiff: diff(gMain, gPlain), savedDiff: diff(gPlain, gSaved) });
    }
  }
  await page.emulateMedia({ colorScheme: "light", reducedMotion: "no-preference" });
  await fresh();
  await page.evaluate((c) => localStorage.setItem("t3board.layout", c), custom);
  await page.reload();
  await page.waitForTimeout(300);
  const wideSaved = await where();
  await page.setViewportSize({ width: 1000, height: 800 });
  await page.waitForTimeout(300);
  const narrowCols = await page.locator(".lcol").count();
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.waitForTimeout(300);
  l5.roundTrip = same(await where(), wideSaved) && narrowCols === 0 && same(wideSaved, [["p-act"], ["p-dec", "p-new"], ["p-tix", "composer"]]);
  l5.pass = l5.views.every((x) => x.identical && x.sameAsMain) && l5.roundTrip;
  out.L5 = l5;

  await fresh();
  const l6 = {};
  const prep = async () => page.evaluate(() => {
    const feed = document.getElementById("feed");
    const it = [...feed.querySelectorAll("li")][8];
    it.setAttribute("tabindex", "-1");
    it.focus({ preventScroll: true });
    feed.scrollTop = 300;
    const open = document.getElementById("open");
    open.scrollTop = 120;
    return { feed: feed.scrollTop, open: open.scrollTop, focus: document.activeElement === it };
  });
  const topItem = () => page.evaluate(() => {
    const feed = document.getElementById("feed"), top = feed.getBoundingClientRect().top;
    return [...feed.querySelectorAll(":scope > li")].findIndex((li) => li.getBoundingClientRect().bottom > top + 1);
  });
  const after = async () => page.evaluate(() => ({ feed: document.getElementById("feed").scrollTop, open: document.getElementById("open").scrollTop, focusInFeed: document.getElementById("feed").contains(document.activeElement) }));
  l6.start = await prep();
  await dragTo("p-dec", (await box(".lcol[data-i='2']")).x + 100, (await box("#p-act")).y + 4);
  l6.neighbourMoved = await after();
  await fresh();
  l6.start2 = await prep();
  await dragTo("p-act", (await box(".lcol[data-i='1']")).x + 200, (await box("#p-tix")).y + 4);
  l6.selfMoved = await after();
  l6.topBefore = await topItem();
  await page.locator(".vsplit").last().focus();
  await page.keyboard.press("ArrowRight");
  await page.locator("#p-new .fold").click();
  await page.waitForTimeout(200);
  l6.topAfterResizeAndCollapse = await topItem();
  const kept = (s, a) => Math.abs(s.feed - a.feed) < 1 && Math.abs(s.open - a.open) < 1;
  l6.pass = kept(l6.start, l6.neighbourMoved) && l6.neighbourMoved.focusInFeed && kept(l6.start2, l6.selfMoved) && l6.selfMoved.focusInFeed && l6.topBefore === l6.topAfterResizeAndCollapse;
  out.L6 = l6;

  const l7 = {};
  await fresh();
  l7.names = await page.evaluate(() => [...document.querySelectorAll(".lctl button, .vsplit, .hsplit:not(.fixed)")].map((b) => [b.getAttribute("aria-label"), b.getAttribute("role") || b.tagName, b.getAttribute("aria-expanded") || b.getAttribute("aria-orientation")]));
  await page.evaluate(() => {
    window.__said = [];
    new MutationObserver(() => { const t = document.getElementById("live").textContent; if (t) window.__said.push(t); }).observe(document.getElementById("live"), { childList: true, characterData: true, subtree: true });
  });
  await page.locator("#p-act .grip").focus();
  await page.keyboard.press("ArrowLeft");
  await page.locator("#p-new .fold").click();
  const db = await head("p-dec").boundingBox();
  await page.mouse.move(db.x + 48, db.y + db.height / 2);
  await page.mouse.down();
  await page.mouse.move(db.x + 300, db.y + 300, { steps: 6 });
  l7.dragging = await page.evaluate(() => [!!document.querySelector(".lghost"), !!document.querySelector(".dropmark"), !!document.querySelector(".lcol.over")]);
  await page.keyboard.press("Escape");
  await page.mouse.up();
  await page.waitForTimeout(300);
  l7.escCancelled = same((await where())[0], ["p-dec"]) && await page.evaluate(() => !document.querySelector(".lghost"));
  l7.said = await page.evaluate(() => window.__said);
  const contrast = await page.evaluate(() => {
    const parse = (c) => { const m = c.match(/[\d.]+/g).map(Number); return m; };
    const lum = ([r, g, b]) => { const f = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b); };
    const ratio = (a, b) => { const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p); return (x + 0.05) / (y + 0.05); };
    const bg = (el) => { for (let n = el; n; n = n.parentElement) { const c = getComputedStyle(n).backgroundColor; const p = parse(c); if (p.length < 4 || p[3] > 0) return p.slice(0, 3); } return [255, 255, 255]; };
    const rows = [];
    for (const b of document.querySelectorAll(".lctl button")) rows.push(["icon", ratio(parse(getComputedStyle(b).color).slice(0, 3), bg(b))]);
    for (const s of document.querySelectorAll(".vsplit, .hsplit")) rows.push(["divider", ratio(parse(getComputedStyle(s).backgroundColor).slice(0, 3), bg(s.parentElement))]);
    const btn = document.querySelector("#lay-reset");
    return { min: { icon: Math.min(...rows.filter((r) => r[0] === "icon").map((r) => r[1])), divider: Math.min(...rows.filter((r) => r[0] === "divider").map((r) => r[1])) } };
  });
  l7.contrastLight = contrast.min;
  await page.emulateMedia({ colorScheme: "dark" });
  await page.waitForTimeout(200);
  l7.contrastDark = (await page.evaluate(() => {
    const parse = (c) => c.match(/[\d.]+/g).map(Number);
    const lum = ([r, g, b]) => { const f = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b); };
    const ratio = (a, b) => { const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p); return (x + 0.05) / (y + 0.05); };
    const bg = (el) => { for (let n = el; n; n = n.parentElement) { const p = parse(getComputedStyle(n).backgroundColor); if (p.length < 4 || p[3] > 0) return p.slice(0, 3); } return [0, 0, 0]; };
    const icon = [...document.querySelectorAll(".lctl button")].map((b) => ratio(parse(getComputedStyle(b).color).slice(0, 3), bg(b)));
    const div = [...document.querySelectorAll(".vsplit, .hsplit")].map((s) => ratio(parse(getComputedStyle(s).backgroundColor).slice(0, 3), bg(s.parentElement)));
    return { icon: Math.min(...icon), divider: Math.min(...div) };
  }));
  await page.emulateMedia({ colorScheme: "light" });
  const sayOnce = (re) => l7.said.filter((t) => re.test(t)).length === 1;
  l7.pass = l7.names.every((n) => n[0] && (n[1] === "BUTTON" ? !/^Collapse/.test(n[0]) || n[2] : n[2])) && same(l7.dragging, [true, true, true]) && l7.escCancelled &&
    sayOnce(/^Activity moved to column 2/) && sayOnce(/^New issues collapsed\.$/) && sayOnce(/^Move cancelled\.$/) &&
    l7.contrastLight.icon >= 3 && l7.contrastDark.icon >= 3 && l7.contrastLight.divider >= 3 && l7.contrastDark.divider >= 3;
  out.L7 = l7;

  out.errors = errs;
  out.summary = Object.fromEntries(Object.entries(out).filter(([k]) => /^L\d/.test(k)).map(([k, v]) => [k, v.pass ? "pass" : "FAIL"]));
  return out;
}
