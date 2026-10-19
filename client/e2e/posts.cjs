// Start `cargo test --locked posts_browser_fixture -- --ignored --nocapture`
// and `cd client && npx vite --host 127.0.0.1 --port 5178`, then run this file
// with Playwright available on NODE_PATH. API/SQLite are real; only failure
// responses and external YouTube pages/images are intercepted.
const { chromium } = require("playwright");
const assert = require("node:assert/strict");
const fs = require("node:fs/promises");
const base = process.env.E2E_BASE_URL || "http://127.0.0.1:5178";
const output = process.env.E2E_OUTPUT || "/tmp/youtube-post-e2e";
const ids = ["p-text", "p-image", "p-gallery"];

async function main() {
  await fs.mkdir(output, { recursive: true });
  const browser = await chromium.launch({ headless: true });
  const results = [];
  try {
    for (const width of [375, 1280, 320]) {
      for (const colorScheme of ["dark", "light"]) {
        const context = await browser.newContext({ viewport: { width, height: 900 }, colorScheme });
        await context.route("https://**/*", (route) =>
          route.fulfill(
            route.request().resourceType() === "document"
              ? { contentType: "text/html", body: "<p>YouTube fixture</p>" }
              : { contentType: "application/javascript", body: "" },
          ),
        );
        const page = await context.newPage();
        const errors = [];
        page.on("pageerror", (error) => errors.push(error.message));
        for (const id of ids)
          assert((await context.request.patch(`${base}/api/posts/${id}/unhide`)).ok());
        assert((await context.request.patch(`${base}/api/videos/p-text/unhide`)).ok());
        await page.goto(base);
        await page.locator(".post-card").first().waitFor();
        assert.equal(await page.locator(".post-card").count(), 3);
        const label = `after-${width}-${colorScheme}`;
        await page.screenshot({ path: `${output}/${label}-feed.png`, fullPage: true });
        const feedText = (await page.locator(".feed").innerText()).length;
        const firstTop = await page
          .locator(".post-card")
          .first()
          .evaluate((node) => node.getBoundingClientRect().top);
        let cardIndex = 0;
        for (const card of await page.locator(".post-card").all()) {
          await card.scrollIntoViewIfNeeded();
          const metrics = await card.evaluate((node) => {
            const button = node.querySelector("button.post-state");
            const a = button.getBoundingClientRect();
            const overlap = [
              ...node.querySelectorAll(".content,.image,.reply,.channel,.time"),
            ].some((item) => {
              const b = item.getBoundingClientRect();
              return (
                Math.min(a.right, b.right) > Math.max(a.left, b.left) &&
                Math.min(a.bottom, b.bottom) > Math.max(a.top, b.top)
              );
            });
            return {
              overlap,
              height: a.height,
              overflow: document.documentElement.scrollWidth > innerWidth,
            };
          });
          assert.equal(metrics.overlap, false);
          assert(metrics.height >= 44);
          assert.equal(metrics.overflow, false);
          await card.screenshot({ path: `${output}/${label}-card-${cardIndex++}.png` });
        }
        // Reply opens YouTube without changing visibility.
        const textCard = page.locator(".post-card").filter({ hasText: "画像なしの投稿です。" });
        const popupPromise = page.waitForEvent("popup");
        await textCard.getByRole("link", { name: "返信" }).click();
        const popup = await popupPromise;
        assert(popup.url().includes("/post/p-text"));
        await popup.close();
        assert.equal(await page.locator(".post-card").count(), 3);
        // A failed hide keeps the card and permits retry.
        await page.route("**/api/posts/p-text/hide", (route) =>
          route.fulfill({ status: 500, body: "{}" }),
        );
        await textCard.getByRole("button", { name: "もう見た" }).click();
        await page.getByText("500 Internal Server Error", { exact: true }).waitFor();
        assert.equal(await textCard.count(), 1);
        await page.unroute("**/api/posts/p-text/hide");
        // Keyboard focus and activation, with successful one-tap removal.
        const hide = textCard.getByRole("button", { name: "もう見た" });
        await hide.focus();
        await page.keyboard.press("Tab");
        await page.keyboard.press("Shift+Tab");
        assert.equal(await hide.evaluate((node) => getComputedStyle(node).outlineStyle), "solid");
        await hide.press("Enter");
        await page.getByText("非表示にしました", { exact: true }).waitFor();
        await textCard.waitFor({ state: "detached" });
        assert.equal(await page.locator(".video-card").count(), 1, "same ID video stays visible");
        for (const text of ["画像付きの投稿です。", "複数画像の長い投稿です。"]) {
          const card = page.locator(".post-card").filter({ hasText: text });
          await card.getByRole("button", { name: "もう見た" }).click();
          await card.waitFor({ state: "detached" });
        }
        await page.reload();
        await page.locator(".video-card").waitFor();
        assert.equal(await page.locator(".post-card").count(), 0);
        await page.getByRole("button", { name: "もう見た" }).click();
        await page.locator(".video-card").waitFor({ state: "detached" });
        // Navigate through the actual menu.
        await page.getByRole("button", { name: "メニュー" }).click();
        await page.getByRole("link", { name: "視聴履歴" }).click();
        await page.locator(".post-card").first().waitFor();
        assert.equal(await page.locator(".post-card").count(), 3);
        assert.equal(
          await page.locator(".video-card").count(),
          1,
          "mixed history permits identical IDs",
        );
        const historyHeaderText = (await page.locator(".history-header").innerText()).length;
        await page.screenshot({ path: `${output}/${label}-history.png`, fullPage: true });
        const historyCard = page.locator(".post-card").filter({ hasText: "画像なしの投稿です。" });
        await page.route("**/api/posts/p-text/unhide", (route) =>
          route.fulfill({ status: 500, body: "{}" }),
        );
        await historyCard.getByRole("button", { name: "復元", exact: true }).click();
        await page.getByText("500 Internal Server Error", { exact: true }).waitFor();
        assert.equal(await historyCard.count(), 1);
        await page.unroute("**/api/posts/p-text/unhide");
        for (const text of [
          "画像なしの投稿です。",
          "画像付きの投稿です。",
          "複数画像の長い投稿です。",
        ]) {
          const card = page.locator(".post-card").filter({ hasText: text });
          await card.getByRole("button", { name: "復元", exact: true }).click();
          await card.waitFor({ state: "detached" });
        }
        assert.equal(await page.locator(".video-card").count(), 1);
        assert((await context.request.patch(`${base}/api/videos/p-text/unhide`)).ok());
        await page.reload();
        await page.getByText("視聴履歴はまだありません").waitFor();
        await page.goto(base);
        await page.locator(".post-card").first().waitFor();
        assert.equal(await page.locator(".post-card").count(), 3);
        assert.deepEqual(errors, []);
        results.push({
          width,
          colorScheme,
          feedText,
          firstTop,
          historyHeaderText,
          hideClicks: 1,
          restoreClicks: 1,
          lifecycle: "passed",
        });
        await context.close();
      }
    }
  } finally {
    await browser.close();
  }
  await fs.writeFile(`${output}/after.json`, JSON.stringify(results, null, 2));
  console.log(JSON.stringify(results, null, 2));
}
main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
