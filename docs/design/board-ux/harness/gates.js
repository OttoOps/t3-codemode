async (page) => {
  const base = "__BOARD_URL__";
  const out = "__CAPTURE_DIR__";
  const r = {};
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  const shot = (name) => page.screenshot({ path: `${out}/${name}.png` });
  const open = async (w, h, scheme) => {
    await page.emulateMedia({ colorScheme: scheme || "light", reducedMotion: "reduce" });
    await page.setViewportSize({ width: w, height: h });
    await page.goto(base);
    await page.waitForSelector("#cards .card");
    await page.waitForTimeout(800);
  };

  const fits = () => page.evaluate(() => {
    const s = document.scrollingElement;
    return { scrollHeight: s.scrollHeight, innerHeight, scrollWidth: s.scrollWidth, innerWidth,
      ok: s.scrollHeight <= innerHeight && s.scrollWidth <= innerWidth };
  });

  r.U1 = {};
  for (const [w, h] of [[1440, 900], [1280, 800], [1024, 768]]) {
    await open(w, h);
    r.U1[`${w}x${h}`] = { ...(await fits()), ...(await page.evaluate(() => ({
      decisions: document.querySelectorAll("#cards .card:not(.gone)").length,
      openTickets: document.querySelectorAll("#open .tk:not(.gone)").length,
    }))) };
  }

  r.U2 = {};
  for (const [w, h] of [[1440, 900], [1280, 800], [1024, 768]]) {
    await open(w, h);
    r.U2[`${w}x${h}`] = await page.evaluate(() => {
      for (const id of ["cards", "open", "feed", "out"]) { const el = document.getElementById(id); el.scrollTop = el.scrollHeight; }
      const inView = (id) => {
        const b = document.getElementById(id).getBoundingClientRect();
        return b.width > 0 && b.top >= 0 && b.left >= 0 && b.bottom <= innerHeight && b.right <= innerWidth;
      };
      return { launch: inView("issues"), ask: inView("question"), pageScrollY: scrollY };
    });
  }
  await open(1024, 768);
  await page.fill("#issues", "acme/widgets#52");
  await page.click("#mode-ask");
  await page.fill("#question", "what waits on me");
  await page.click("#mode-launch");
  r.U2.switchKeepsDrafts = await page.evaluate(() => ({
    launch: document.getElementById("issues").value, ask: document.getElementById("question").value,
    focused: document.activeElement.id,
  }));
  await open(1440, 900);
  const tabsTo = async (id) => {
    await page.goto(base);
    await page.waitForSelector("#cards .card");
    for (let i = 1; i <= 8; i++) {
      await page.keyboard.press("Tab");
      const a = await page.evaluate(() => document.activeElement.textContent.trim());
      if (a === `Skip to ${id}`) { await page.keyboard.press("Enter"); return { tabs: i, focus: await page.evaluate(() => document.activeElement.id) }; }
    }
    return { tabs: null };
  };
  r.U2.tabPresses = { launch: await tabsTo("Launch"), ask: await tabsTo("Ask") };
  await page.keyboard.press("Escape");
  await page.keyboard.press("l");
  r.U2.shortcutL = await page.evaluate(() => document.activeElement.id);
  await page.keyboard.press("Escape");
  await page.keyboard.press("/");
  r.U2.shortcutSlash = await page.evaluate(() => document.activeElement.id);

  await open(1440, 900);
  r.U5 = await page.evaluate(() => {
    const list = document.getElementById("cards").getBoundingClientRect();
    const hs = [...document.querySelectorAll("#cards .card")].map((c) => c.getBoundingClientRect());
    const vis = hs.filter((b) => b.top >= list.top && b.bottom <= list.bottom).length;
    const sorted = hs.map((b) => b.height).sort((a, b) => a - b);
    return { visible1440: vis, median: sorted[Math.floor(sorted.length / 2)], heights: sorted };
  });
  r.U6 = {};
  for (const [w, h] of [[1440, 900], [1024, 768]]) {
    await open(w, h);
    r.U6[w] = await page.evaluate(() => {
      const list = document.getElementById("open").getBoundingClientRect();
      const rows = [...document.querySelectorAll("#open > .tk")].map((el) => ({ el, b: el.getBoundingClientRect() }));
      const vis = rows.filter(({ b }) => b.height > 0 && b.top >= list.top && b.bottom <= list.bottom).length;
      const sig = (el) => [!!el.querySelector('[data-part="note"]:not([hidden])'), el.querySelectorAll(".wl .wi").length > 0].join();
      const bySig = {};
      for (const { el, b } of rows) (bySig[sig(el)] ||= new Set()).add(Math.round(b.height));
      return { visible: vis, heightsBySameFields: Object.fromEntries(Object.entries(bySig).map(([k, v]) => [k, [...v]])) };
    });
  }

  await open(1440, 900);
  r.U7 = await page.evaluate(() => {
    const kinds = [...document.querySelectorAll("#cards .card")].map((c) => c.dataset.kind);
    const rank = { blocker: 0, error: 0, escalate: 0, gate: 1, report: 2 };
    const sorted = kinds.every((k, i) => i === 0 || (rank[kinds[i - 1]] ?? 2) <= (rank[k] ?? 2));
    return { kinds, sorted };
  });
  const needs = page.locator("#open .needs:visible").first();
  const needId = (await needs.getAttribute("aria-label")).match(/card (\w+)/)[1];
  await needs.click();
  r.U7.rowToCard = await page.evaluate((id) => document.activeElement.dataset.id === id, needId);
  await page.locator(`#cards [data-id="${needId}"] [data-part="id"]`).click();
  r.U7.cardToRow = await page.evaluate(() => document.activeElement.classList.contains("tk"));
  await page.emulateMedia({ colorScheme: "light", reducedMotion: "reduce" });
  await page.evaluate(() => { document.documentElement.style.filter = "grayscale(1)"; });
  await shot("grayscale-1440");
  await page.evaluate(() => { document.documentElement.style.filter = ""; });

  await open(1440, 900);
  const sent = [];
  await page.route("**/api/cards/*/answer", (route) => { sent.push(route.request().url()); route.fulfill({ status: 200, contentType: "application/json", body: "{}" }); });
  r.U8 = {};
  r.U8.optionStyles = await page.evaluate(() => {
    const st = [...document.querySelectorAll("#cards .act button[data-opt]")].map((b) => { const c = getComputedStyle(b); return c.backgroundColor + "|" + c.color + "|" + c.fontWeight; });
    return { distinct: [...new Set(st)].length, any: st.length };
  });
  const opt = page.locator("#cards .card button[data-opt]").first();
  const optCard = await opt.evaluate((b) => b.closest(".card").dataset.id);
  await opt.click();
  await page.waitForTimeout(4500);
  r.U8.sentBefore5s = sent.length;
  r.U8.sendingText = await page.locator(`#cards [data-id="${optCard}"] [data-part="what"]`).textContent();
  await page.waitForTimeout(1000);
  r.U8.sentAfter5s = sent.length;
  const opt2 = page.locator(`#cards .card:not([data-id="${optCard}"]) button[data-opt]`).first();
  await opt2.click();
  await page.waitForTimeout(1000);
  await page.keyboard.press("Enter");
  await page.waitForTimeout(5500);
  r.U8.afterUndo = sent.length;
  await page.locator("#cards").click({ position: { x: 4, y: 4 } });
  await page.keyboard.press("Escape");
  await page.evaluate(() => [...document.querySelectorAll("#cards .card:not(.gone)")].find((c) => c.querySelector(".act:not([hidden]) [data-opt]")).focus());
  await page.keyboard.press("1");
  await page.waitForTimeout(5500);
  r.U8.digitSent = sent.length - r.U8.afterUndo;
  r.U8.digitFocus = await page.evaluate(() => document.activeElement.dataset.opt || document.activeElement.tagName);
  await page.unroute("**/api/cards/*/answer");

  await open(1440, 900);
  await page.focus("#question");
  await page.keyboard.type("lgj/?");
  r.U9 = { typedInField: await page.inputValue("#question"), focusAfter: await page.evaluate(() => document.activeElement.id) };
  await page.fill("#question", "");
  await page.evaluate(() => document.querySelector("#cards .card").focus());
  await page.keyboard.press("?");
  r.U9.helpOpens = await page.evaluate(() => document.getElementById("keys").open);
  await page.click("#keys-on");
  await page.click("#keys-close");
  await page.evaluate(() => document.querySelector("#cards .card").focus());
  await page.keyboard.press("l");
  r.U9.offStopsShortcuts = await page.evaluate(() => document.activeElement.id !== "issues");
  await page.evaluate(() => localStorage.removeItem("t3board.keys"));
  await page.goto(base);
  await page.waitForSelector("#cards .card");
  await page.evaluate(() => document.querySelector("#cards .card").focus());
  await page.keyboard.press("j");
  r.U9.jMoves = await page.evaluate(() => document.activeElement.dataset.id);
  await page.keyboard.press("g");
  await page.keyboard.press("t");
  r.U9.gtTickets = await page.evaluate(() => document.activeElement.classList.contains("tk"));
  await page.keyboard.press("g");
  await page.keyboard.press("a");
  r.U9.gaActivity = await page.evaluate(() => document.activeElement.parentElement.id);
  r.U9.skipLinks = await page.evaluate(() => [...document.querySelectorAll(".skip a")].map((a) => a.textContent));

  r.U10 = {};
  const touch = () => page.evaluate(() => {
    const small = [];
    for (const el of document.querySelectorAll("a, button, input, textarea, [role=tab]")) {
      const b = el.getBoundingClientRect();
      if (!b.width || !b.height || el.closest(".skip, dialog") || getComputedStyle(el).visibility === "hidden") continue;
      if (b.bottom < 0 || b.top > innerHeight) continue;
      if (b.height < 44 || b.width < 44) small.push((el.textContent || el.id || el.tagName).trim().slice(0, 24) + " " + Math.round(b.width) + "x" + Math.round(b.height));
    }
    const shown = ["p-dec", "p-tix", "p-act"].filter((id) => getComputedStyle(document.getElementById(id)).display !== "none");
    return { small, shownPanels: shown };
  });
  for (const [w, h] of [[768, 1024], [390, 844]]) {
    await open(w, h);
    r.U10[`${w}x${h}`] = { ...(await fits()), ...(await touch()) };
    await shot(`narrow-${w}`);
  }
  await open(195, 422);
  r.U10.zoom200 = { ...(await fits()), clipped: await page.evaluate(() => [...document.querySelectorAll("button, input, textarea, a")]
    .filter((el) => { const b = el.getBoundingClientRect(); return b.width && !el.closest(".skip") && (b.left < 0 || b.right > innerWidth + 0.5); })
    .map((el) => (el.id || el.textContent).trim().slice(0, 20) + "@" + (el.parentElement.id || el.parentElement.className))) };
  await open(390, 844);
  await page.evaluate(() => { document.getElementById("cards").scrollTop = 200; });
  const beforeKb = await page.evaluate(() => ({ panel: document.getElementById("app").dataset.panel, top: document.getElementById("cards").scrollTop }));
  await page.click("#issues");
  await page.evaluate(() => {
    const vv = window.visualViewport;
    Object.defineProperty(vv, "height", { configurable: true, get: () => 460 });
    vv.dispatchEvent(new Event("resize"));
  });
  r.U10.keyboard = await page.evaluate(() => {
    const f = document.getElementById("issues").getBoundingClientRect();
    const b = document.getElementById("launch-btn").getBoundingClientRect();
    return { field: [Math.round(f.top), Math.round(f.bottom)], submit: [Math.round(b.top), Math.round(b.bottom)], within: [f, b].every((x) => x.width > 0 && x.height > 0 && x.top >= 0 && x.bottom <= 460 && x.left >= 0 && x.right <= window.innerWidth), cls: document.getElementById("app").className };
  });
  await page.evaluate(() => {
    Object.defineProperty(window.visualViewport, "height", { configurable: true, get: () => 250 });
    window.visualViewport.dispatchEvent(new Event("resize"));
  });
  r.U10.sheet = await page.evaluate(() => ({ cls: document.getElementById("app").className, height: Math.round(document.getElementById("dock").getBoundingClientRect().height) }));
  await page.evaluate(() => {
    Object.defineProperty(window.visualViewport, "height", { configurable: true, get: () => window.innerHeight });
    window.visualViewport.dispatchEvent(new Event("resize"));
    document.activeElement.blur();
  });
  await page.waitForTimeout(300);
  const afterKb = await page.evaluate(() => ({ panel: document.getElementById("app").dataset.panel, top: document.getElementById("cards").scrollTop, cls: document.getElementById("app").className }));
  r.U10.restore = { before: beforeKb, after: afterKb, same: beforeKb.panel === afterKb.panel && beforeKb.top === afterKb.top && !/\b(kb|sheet)\b/.test(afterKb.cls) };

  const contrast = () => page.evaluate(() => {
    const parse = (c) => { const m = c.match(/[\d.]+/g).map(Number); return m.length > 3 ? m : [...m, 1]; };
    const lum = ([r, g, b]) => { const f = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b); };
    const ratio = (a, b) => { const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p); return (x + 0.05) / (y + 0.05); };
    const bgOf = (el) => { for (let n = el; n; n = n.parentElement) { const c = parse(getComputedStyle(n).backgroundColor); if (c[3] > 0.5) return c; } return [255, 255, 255, 1]; };
    const worst = {};
    const note = (k, v) => { worst[k] = Math.min(worst[k] ?? 99, +v.toFixed(2)); };
    for (const el of document.querySelectorAll(".mark, .tag, .needs, .card .err")) {
      if (!el.getClientRects().length || !el.textContent.trim()) continue;
      note("status:" + [...el.classList].join("."), ratio(parse(getComputedStyle(el).color), bgOf(el)));
    }
    for (const el of document.querySelectorAll("input[type=text], textarea")) {
      if (!el.getClientRects().length) continue;
      note("inputBorder", ratio(parse(getComputedStyle(el).borderTopColor), bgOf(el.parentElement)));
    }
    const acc = parse(getComputedStyle(document.querySelector("#cards a, #open a")).color);
    note("focusRing:panel", ratio(acc, parse(getComputedStyle(document.getElementById("p-dec")).backgroundColor)));
    note("focusRing:canvas", ratio(acc, parse(getComputedStyle(document.getElementById("cards")).backgroundColor)));
    for (const el of document.querySelectorAll(".meta, .tn, .cid, .age, #feed time")) {
      if (!el.getClientRects().length || !el.textContent.trim()) continue;
      note("meta", ratio(parse(getComputedStyle(el).color), bgOf(el)));
    }
    return worst;
  });
  r.P1 = {};
  for (const scheme of ["light", "dark"]) {
    await open(1440, 900, scheme);
    await page.click("#tab-settled");
    const s1 = await contrast();
    await page.click("#tab-open");
    r.P1[scheme] = { ...s1, ...(await contrast()) };
  }

  await open(1440, 900);
  r.P2 = await page.evaluate(() => {
    const sizes = new Set(), weights = new Set(), spacing = new Set();
    const allowed = new Set([0, 4, 8, 12, 16, 24]);
    for (const el of document.querySelectorAll("body *")) {
      if (!el.getClientRects().length || el.closest(".skip, dialog, .sr")) continue;
      const cs = getComputedStyle(el);
      const hasText = [...el.childNodes].some((n) => n.nodeType === 3 && n.textContent.trim());
      if ((hasText || el.matches("input, textarea")) && !/mono/i.test(cs.fontFamily)) { sizes.add(cs.fontSize); weights.add(cs.fontWeight); }
      for (const p of ["paddingTop", "paddingRight", "paddingBottom", "paddingLeft", "rowGap", "columnGap"]) {
        const v = cs[p];
        if (v === "normal" || v === "auto") continue;
        const n = Math.round(parseFloat(v) * 100) / 100;
        if (!allowed.has(n)) spacing.add(p + ":" + v + "@" + (el.id || el.className || el.tagName));
      }
    }
    const rules = [];
    const walk = (list) => { for (const rule of list) { if (rule.cssRules) walk(rule.cssRules); if (rule.style) rules.push(rule); } };
    for (const sheet of document.styleSheets) walk(sheet.cssRules);
    for (const rule of rules) {
      for (const prop of rule.style) {
        if (!/^(margin|padding|gap|row-gap|column-gap)/.test(prop)) continue;
        for (const tok of rule.style.getPropertyValue(prop).split(/\s+/)) {
          if (tok === "auto" || tok === "0" || tok === "") continue;
          const n = parseFloat(tok);
          if (!tok.endsWith("px") || !allowed.has(n)) spacing.add("css " + rule.selectorText + " " + prop + ":" + tok);
        }
      }
    }
    return { fontSizes: [...sizes], weights: [...weights], offScale: [...spacing].slice(0, 20) };
  });
  r.P3 = await page.evaluate(() => {
    const acc = getComputedStyle(document.querySelector("#cards a, #open a")).color;
    const need = getComputedStyle(document.querySelector(".s-need") || document.body).color;
    const accentStatus = [...document.querySelectorAll(".mark, .tag")].filter((el) => getComputedStyle(el).color === acc).length;
    const amberOff = [...document.querySelectorAll("body *")].filter((el) => el.getClientRects().length && getComputedStyle(el).color === need
      && !el.closest(".s-need, .needs")).map((el) => el.className).slice(0, 10);
    const ink = getComputedStyle(document.body).color;
    const running = [...document.querySelectorAll(".s-run")].map((m) => getComputedStyle(m).color === ink ? "ink" : getComputedStyle(m).color);
    return { accentStatus, amberOff, runningRoles: [...new Set(running)] };
  });

  r.N1 = await page.evaluate(() => {
    const bad = [];
    for (const el of document.querySelectorAll("body *")) {
      const cs = getComputedStyle(el);
      if (cs.backgroundImage !== "none") bad.push("gradient:" + el.className);
      if (cs.boxShadow !== "none" && el.matches(".card, .pane, section, .tk, .ask")) bad.push("shadow:" + el.className);
    }
    const emoji = /\p{Extended_Pictographic}/u.test(document.body.innerText);
    return { bad: bad.slice(0, 10), emoji };
  });
  r.N2 = await page.evaluate(() => {
    const text = document.body.innerText;
    const zone = (document.getElementById("svc-text").textContent.match(/checked \d\d:\d\d ([A-Z]{2,5})/) || ["", ""])[1];
    const zoneCount = zone ? (text.match(new RegExp("\\b" + zone + "\\b", "g")) || []).length : 0;
    const cut = [...document.querySelectorAll("body *")].filter((el) => {
      if (!el.getClientRects().length || el.children.length) return false;
      const cs = getComputedStyle(el);
      return (cs.textOverflow === "ellipsis" || cs.webkitLineClamp !== "none") && (el.scrollWidth > el.clientWidth + 1 || el.scrollHeight > el.clientHeight + 1);
    });
    const untitled = cut.filter((el) => !el.title && !el.closest("[title]") && !el.closest(".card")).map((el) => el.textContent.slice(0, 30));
    return { markdown: /\*\*|__/.test(text), zone, zoneCount, truncated: cut.length, untitled: untitled.slice(0, 10) };
  });

  r.F4 = await page.evaluate(async () => {
    document.getElementById("tab-done").click();
    document.getElementById("tab-settled").click();
    document.getElementById("tab-open").click();
    const name = (el) => el.getAttribute("aria-label") || el.textContent.trim();
    const sets = {
      options: [...document.querySelectorAll("#cards button[data-opt]")],
      send: [...document.querySelectorAll('#cards [data-part="send"]')],
      dismiss: [...document.querySelectorAll('#cards [data-part="dismiss"]')],
      settle: [...document.querySelectorAll('#done [data-part="settle"], #asks [data-part="settle"]')],
      restore: [...document.querySelectorAll('#settled [data-part="restore"]')],
    };
    const out = {};
    for (const [k, els] of Object.entries(sets)) {
      const names = els.map(name);
      out[k] = { n: els.length, unique: new Set(names).size, contextless: names.filter((n) => !/\b(a\d+|q\d+|[\w.-]+#\d+)\b|“/.test(n)).slice(0, 3) };
    }
    return out;
  });
  r.errors = errors;
  return r;
}
