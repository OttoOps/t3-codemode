async (page) => {
  const base = "__BOARD_URL__";
  const out = "__CAPTURE_DIR__";
  const shot = async (name, opts) => page.screenshot({ path: `${out}/${name}.png`, ...opts });
  const settle = () => page.waitForTimeout(1200);
  const report = {};

  await page.emulateMedia({ colorScheme: "light", reducedMotion: "reduce" });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(base);
  await page.waitForSelector(".card");
  await settle();
  await shot("desktop-1440-light-top");
  await shot("desktop-1440-light-full", { fullPage: true });
  report.pageHeight1440 = await page.evaluate(() => document.documentElement.scrollHeight);

  await page.evaluate(() => { document.getElementById("feed-box").open = true; document.getElementById("settled-box").open = true; });
  await settle();
  report.pageHeightOpen1440 = await page.evaluate(() => document.documentElement.scrollHeight);
  await shot("desktop-1440-light-expanded-full", { fullPage: true });
  await page.evaluate(() => { document.getElementById("feed-box").open = false; document.getElementById("settled-box").open = false; });

  report.launchBelowFold = await page.evaluate(() => {
    const r = document.getElementById("issues").getBoundingClientRect();
    return { top: Math.round(r.top + scrollY), viewport: innerHeight, screensDown: +((r.top + scrollY) / innerHeight).toFixed(2) };
  });

  await page.locator("#issues").scrollIntoViewIfNeeded();
  await page.locator("#issues").click();
  await page.keyboard.type("acme/widgets#52");
  const samples = [];
  for (let i = 0; i < 16; i++) {
    samples.push(await page.evaluate(() => {
      const r = document.activeElement.getBoundingClientRect();
      return { t: Date.now(), id: document.activeElement.id, top: Math.round(r.top), scrollY: Math.round(scrollY),
               height: document.documentElement.scrollHeight, cards: document.querySelectorAll(".card").length };
    }));
    await page.waitForTimeout(2500);
  }
  report.focusedInputDrift = samples;
  const tops = samples.map((s) => s.top);
  report.focusedInputDriftPx = Math.max(...tops) - Math.min(...tops);
  await shot("desktop-1440-light-typing-launch");

  await page.evaluate(() => scrollTo(0, 0));
  const cardInput = page.locator(".card input[type=text]").first();
  await cardInput.click();
  await page.keyboard.type("hold the deploy until");
  const cardSamples = [];
  for (let i = 0; i < 10; i++) {
    cardSamples.push(await page.evaluate(() => {
      const r = document.activeElement.getBoundingClientRect();
      return { tag: document.activeElement.tagName, top: Math.round(r.top), value: document.activeElement.value };
    }));
    await page.waitForTimeout(2500);
  }
  report.cardReplyDrift = cardSamples;
  await shot("desktop-1440-light-typing-card");

  await page.emulateMedia({ colorScheme: "dark", reducedMotion: "reduce" });
  await page.evaluate(() => scrollTo(0, 0));
  await settle();
  await shot("desktop-1440-dark-top");

  await page.emulateMedia({ colorScheme: "light", reducedMotion: "reduce" });
  await page.setViewportSize({ width: 1024, height: 768 });
  await page.goto(base);
  await page.waitForSelector(".card");
  await settle();
  await shot("tablet-1024-light-top");

  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(base);
  await page.waitForSelector(".card");
  await settle();
  await shot("phone-390-light-top");
  await shot("phone-390-light-full", { fullPage: true });
  report.phone = await page.evaluate(() => ({
    scrollWidth: document.documentElement.scrollWidth, innerWidth,
    pageHeight: document.documentElement.scrollHeight,
    launchScreensDown: +((document.getElementById("issues").getBoundingClientRect().top + scrollY) / innerHeight).toFixed(2),
  }));
  return report;
}
