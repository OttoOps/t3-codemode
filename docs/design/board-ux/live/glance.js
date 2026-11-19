async (page) => {
  const base = "__BOARD_URL__";
  const now = "__NOW__";
  const report = { base, now, views: {} };

  await page.route("**/*", (route) => {
    if (route.request().method() !== "GET") return route.abort();
    return route.continue();
  });
  if (now) await page.clock.setFixedTime(new Date(now));

  const probe = () => page.evaluate(() => {
    const rgb = (s) => {
      const m = /rgba?\(([\d.]+),\s*([\d.]+),\s*([\d.]+)(?:,\s*([\d.]+))?\)/.exec(s || "");
      return m ? { r: +m[1], g: +m[2], b: +m[3], a: m[4] === undefined ? 1 : +m[4] } : null;
    };
    const lum = (c) => {
      const f = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4); };
      return 0.2126 * f(c.r) + 0.7152 * f(c.g) + 0.0722 * f(c.b);
    };
    const ratio = (a, b) => { const x = lum(a), y = lum(b); return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05); };
    const over = (top, under) => ({ r: top.r * top.a + under.r * (1 - top.a), g: top.g * top.a + under.g * (1 - top.a), b: top.b * top.a + under.b * (1 - top.a), a: 1 });
    const bgOf = (el) => {
      const stack = [];
      for (let n = el; n && n.nodeType === 1; n = n.parentElement) {
        const c = rgb(getComputedStyle(n).backgroundColor);
        if (c && c.a > 0) { stack.push(c); if (c.a >= 1) break; }
      }
      let out = { r: 255, g: 255, b: 255, a: 1 };
      for (let i = stack.length - 1; i >= 0; i--) out = over(stack[i], out);
      return out;
    };
    const pover = (t, u) => ({ r: t.r + u.r * (1 - t.a), g: t.g + u.g * (1 - t.a), b: t.b + u.b * (1 - t.a), a: t.a + u.a * (1 - t.a) });
    const pm = (c) => ({ r: c.r * c.a, g: c.g * c.a, b: c.b * c.a, a: c.a });
    const fade = (c, o) => ({ r: c.r * o, g: c.g * o, b: c.b * o, a: c.a * o });
    const paint = (el, top) => {
      const chain = [];
      for (let n = el; n && n.nodeType === 1; n = n.parentElement) chain.unshift(n);
      const group = (i) => {
        let acc = { r: 0, g: 0, b: 0, a: 0 };
        for (let j = i; j < chain.length; j++) {
          if (j > i && +getComputedStyle(chain[j]).opacity < 1) return fade(pover(group(j), acc), +getComputedStyle(chain[i]).opacity);
          const c = rgb(getComputedStyle(chain[j]).backgroundColor);
          if (c && c.a > 0) acc = pover(pm(c), acc);
        }
        if (top) acc = pover(pm(top), acc);
        return fade(acc, +getComputedStyle(chain[i]).opacity);
      };
      const out = pover(group(0), { r: 255, g: 255, b: 255, a: 1 });
      return { r: out.r, g: out.g, b: out.b, a: 1 };
    };
    const shown = (el) => {
      const r = el.getBoundingClientRect();
      if (!r.width || !r.height || r.bottom < 0 || r.top > innerHeight || r.right < 0 || r.left > innerWidth) return false;
      if (el.closest("[hidden], .sr")) return false;
      for (let a = el.parentElement; a && a !== document.body; a = a.parentElement) {
        if (getComputedStyle(a).overflowY === "visible") continue;
        const c = a.getBoundingClientRect();
        if (r.bottom <= c.top || r.top >= c.bottom || r.right <= c.left || r.left >= c.right) return false;
      }
      const cs = getComputedStyle(el);
      return cs.visibility !== "hidden" && cs.display !== "none";
    };
    const own = (el) => [...el.childNodes].some((n) => n.nodeType === 3 && n.textContent.trim());
    const all = [...document.querySelectorAll("body *")].filter(shown);
    const tok = (k) => {
      const d = document.createElement("i");
      d.style.color = `var(${k})`;
      document.body.append(d);
      const c = rgb(getComputedStyle(d).color);
      d.remove();
      return c;
    };
    const NEED = tok("--need"), FILL = tok("--need-fill"), FAIL = tok("--fail"), ACC = tok("--accent"), GO = tok("--go");
    const same = (a, b) => a && b && Math.abs(a.r - b.r) < 2 && Math.abs(a.g - b.g) < 2 && Math.abs(a.b - b.b) < 2;
    const desc = (el) => el.tagName.toLowerCase() + (el.id ? "#" + el.id : "") + (el.className && typeof el.className === "string" ? "." + el.className.trim().split(/\s+/).join(".") : "");

    const G11 = { text: [], lines: [], min: { text: 99, line: 99 } };
    for (const el of all) {
      const cs = getComputedStyle(el);
      if (own(el) || el.matches("input, textarea")) {
        const bg = paint(el, null);
        const eff = paint(el, rgb(cs.color));
        const big = parseFloat(cs.fontSize) >= 24 || (parseFloat(cs.fontSize) >= 18.66 && +cs.fontWeight >= 700);
        const r = ratio(eff, bg);
        G11.min.text = Math.min(G11.min.text, r);
        if (r < (big ? 3 : 4.5)) G11.text.push({ el: desc(el), text: el.textContent.trim().slice(0, 40), ratio: +r.toFixed(2) });
        if (el.matches("input, textarea") && el.placeholder && !el.value) {
          const pcs = getComputedStyle(el, "::placeholder");
          const ph = rgb(pcs.color);
          const pe = paint(el, { ...ph, a: ph.a * (+pcs.opacity || 1) });
          const pr = ratio(pe, bg);
          G11.min.text = Math.min(G11.min.text, pr);
          if (pr < 4.5) G11.text.push({ el: desc(el) + "::placeholder", ratio: +pr.toFixed(2) });
        }
      }
      for (const side of ["Top", "Right", "Bottom", "Left"]) {
        if (parseFloat(cs[`border${side}Width`]) < 1 || cs[`border${side}Style`] === "none") continue;
        const bc = rgb(cs[`border${side}Color`]);
        if (!bc || bc.a === 0) continue;
        const under = el.parentElement ? bgOf(el.parentElement) : { r: 255, g: 255, b: 255, a: 1 };
        const inside = bgOf(el);
        const filled = same(bc, rgb(cs.backgroundColor));
        const r = filled ? ratio(over(bc, under), under) : Math.min(ratio(over(bc, under), under), ratio(over(bc, inside), inside));
        G11.min.line = Math.min(G11.min.line, r);
        if (r < 3) G11.lines.push({ el: desc(el), side, ratio: +r.toFixed(2) });
      }
    }
    G11.min.text = +G11.min.text.toFixed(2);
    G11.min.line = +G11.min.line.toFixed(2);

    const G4 = { amberOff: [], amberWithoutCard: [], redOff: [], accentOff: [] };
    for (const el of all) {
      const cs = getComputedStyle(el);
      const fg = rgb(cs.color), bg = rgb(cs.backgroundColor);
      const amber = (same(fg, NEED) && own(el)) || same(bg, FILL) || (same(fg, NEED) && el.matches(".mark"));
      if (amber) {
        if (!el.closest(".s-need, .needs, .pill.on")) G4.amberOff.push(desc(el));
        const tk = el.closest(".tk, .card, .tgroup, .counts, .ph, .mtab");
        if (tk && tk.matches(".tk") && tk.dataset.state !== "need") G4.amberWithoutCard.push(tk.dataset.thread);
      }
      if ((same(fg, FAIL) && (own(el) || el.matches(".mark"))) || (same(rgb(cs.borderTopColor), FAIL) && parseFloat(cs.borderTopWidth) > 0)) {
        if (!el.closest(".s-fail, .needs.r-fail, .k-fail, .err, .err-strip, .perr, .bad, .newbtn[data-urgent], .stale .conn")) G4.redOff.push(desc(el));
      }
      if (same(fg, ACC) && own(el) && !el.closest("a, .linkish, .newbtn, button.btn")) G4.accentOff.push(desc(el));
      if (same(bg, GO) && !el.matches(".btn.go, .btn.answer")) G4.accentOff.push(desc(el));
    }

    const text = (sel) => [...document.querySelectorAll(sel)].filter(shown).map((e) => e.textContent.trim());
    const G7 = [...text(".tl"), ...text(".ttl"), ...text(".wi"), ...text(".ctx .lbl"), ...text(".f1 .who")]
      .filter((t) => /^gate:|\w_\w|\b[0-9a-f]{40}\b|\b[0-9a-f]{64}\b/.test(t));

    const visibleText = all.filter(own).map((el) => [...el.childNodes].filter((n) => n.nodeType === 3).map((n) => n.textContent).join(" "));
    const G8 = {
      clockTimes: visibleText.filter((t) => /\b\d{1,2}:\d{2}\b/.test(t)).slice(0, 10),
      zones: visibleText.filter((t) => /\b(MST|MDT|UTC|[A-Z]{2}T)\b/.test(t)).length,
    };

    const rows = [...document.querySelectorAll("#feed > li")].filter(shown);
    const msgs = rows.map((li) => (li.querySelector(".msg") || {}).textContent || "");
    const labels = rows.map((li) => (li.querySelector(".who") || {}).textContent + "|" + (li.querySelector(".fsub") || {}).textContent);
    const G9 = {
      rows: rows.length,
      kindPrefix: msgs.filter((m) => /^(event|note|gate|report|blocker):/i.test(m)).length,
      longHash: msgs.filter((m) => /\b[0-9a-f]{8,}\b/.test(m)).length,
      repeatedNeighbours: labels.filter((l, i) => i && l === labels[i - 1]).length,
      groups: rows.filter((li) => { const m = li.querySelector(".more"); return m && !m.hidden; }).length,
    };

    const out = document.getElementById("out");
    const G10 = { answersHeight: Math.round(out.getBoundingClientRect().height), feedTop: Math.round(document.getElementById("feed").getBoundingClientRect().top) };
    return { G4, G7, G8, G9, G10, G11 };
  });

  const views = [
    { name: "1920-dark", w: 1920, h: 1080, scheme: "dark" },
    { name: "1920-light", w: 1920, h: 1080, scheme: "light" },
    { name: "1366-dark", w: 1366, h: 768, scheme: "dark" },
    { name: "1366-light", w: 1366, h: 768, scheme: "light" },
    { name: "390-dark", w: 390, h: 844, scheme: "dark" },
    { name: "390-light", w: 390, h: 844, scheme: "light" },
  ];
  for (const v of views) {
    await page.emulateMedia({ colorScheme: v.scheme, reducedMotion: "reduce" });
    await page.setViewportSize({ width: v.w, height: v.h });
    await page.goto(base);
    await page.waitForSelector(".card");
    await page.waitForTimeout(1000);
    report.views[v.name] = await probe();
    if (v.w < 700) {
      for (const panel of ["tickets", "activity"]) {
        await page.click(`.mtab[data-panel="${panel}"]`);
        await page.waitForTimeout(300);
        report.views[`${v.name}-${panel}`] = await probe();
      }
    }
  }
  return JSON.stringify(report, null, 1);
}
