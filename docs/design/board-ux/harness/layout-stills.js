async (page) => {
  const base = "__BOARD_URL__";
  const out = "__CAPTURE_DIR__";
  const blank = () => page.evaluate(() => {
    document.title = "";
    for (const h of document.querySelectorAll("h1")) h.textContent = "";
  });
  const shot = async (name) => { await blank(); await page.screenshot({ path: `${out}/${name}.png` }); };
  const open = async (w, h, scheme, layout) => {
    await page.emulateMedia({ colorScheme: scheme, reducedMotion: "reduce" });
    await page.setViewportSize({ width: w, height: h });
    await page.goto(base);
    await page.evaluate((l) => { localStorage.clear(); if (l) localStorage.setItem("t3board.layout", JSON.stringify(l)); }, layout || null);
    await page.reload();
    await page.waitForTimeout(800);
    await page.mouse.move(w - 2, h - 2);
  };
  const head = (id) => page.locator(id === "composer" ? "#launch .ph" : "#" + id + " > .ph");
  const press = async (id) => {
    const b = await head(id).boundingBox();
    await page.mouse.move(b.x + 48, b.y + b.height / 2);
    await page.mouse.down();
    await page.mouse.move(b.x + 64, b.y + b.height / 2 + 8, { steps: 3 });
  };
  const box = (sel) => page.locator(sel).first().boundingBox();
  const rearranged = { cols: [{ w: 520, p: ["act"] }, { w: 900, p: ["dec", "tix"] }, { w: 420, p: ["new", "cmd"] }], h: { dec: 420, tix: 520 }, c: { new: true } };

  await open(1600, 1000, "light");
  await shot("rest");

  await press("p-dec");
  const tix = await box("#p-tix");
  await page.mouse.move(tix.x + tix.width / 2, tix.y + 6, { steps: 10 });
  await page.waitForTimeout(150);
  await shot("drag-into-column");
  await page.mouse.up();
  await page.waitForTimeout(300);

  await press("p-act");
  const vacant = await box(".lcol.vacant");
  await page.mouse.move(vacant.x + vacant.width / 2, vacant.y + 300, { steps: 10 });
  await page.waitForTimeout(150);
  await shot("drag-to-empty-column");
  await page.mouse.up();

  await open(1600, 1000, "light");
  const v = await box(".vsplit");
  await page.mouse.move(v.x + 0.5, v.y + 400);
  await page.waitForTimeout(600);
  await shot("divider-hover");
  const hs = await box(".lcol[data-i='2'] > .hsplit:not(.fixed)");
  await page.mouse.move(hs.x + 150, hs.y + 0.5);
  await page.mouse.down();
  await page.mouse.move(hs.x + 150, hs.y + 160.5, { steps: 6 });
  await page.waitForTimeout(150);
  await shot("divider-resize");
  await page.mouse.up();

  await open(1600, 1000, "light", { cols: [{ w: null, p: ["dec"] }, { w: null, p: ["new", "tix"] }, { w: null, p: ["cmd", "act"] }], h: {}, c: { new: true, cmd: true } });
  await shot("collapsed");

  await open(1920, 1080, "light", rearranged);
  await shot("rearranged-light");
  await open(1920, 1080, "dark", rearranged);
  await shot("rearranged-dark");
  await page.evaluate(() => localStorage.clear());
  return "ok";
}
