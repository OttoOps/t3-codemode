async (page) => {
  const out = "__CAPTURE_DIR__";
  await page.emulateMedia({ colorScheme: "light", reducedMotion: "reduce" });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("__EMPTY_URL__");
  await page.waitForSelector(".empty");
  await page.waitForTimeout(1000);
  await page.screenshot({ path: `${out}/desktop-1440-light-empty.png` });
  await page.route("**/api/state", () => {});
  await page.route("**/api/events", () => {});
  await page.goto("__BOARD_URL__", { waitUntil: "domcontentloaded" });
  await page.waitForTimeout(800);
  await page.screenshot({ path: `${out}/desktop-1440-light-loading.png` });
  await page.unrouteAll({ behavior: "ignoreErrors" });
  await page.route("**/api/events", (r) => r.abort());
  await page.goto("__BOARD_URL__");
  await page.waitForSelector(".banner.on");
  await page.waitForTimeout(800);
  await page.screenshot({ path: `${out}/desktop-1440-light-disconnected.png` });
  await page.unrouteAll({ behavior: "ignoreErrors" });
  return "ok";
}
