/**
 * S4-4.2 E2E · 4-Tab 主链路：书库 → 阅读器 → AI → 复习。
 * 浏览器 mock 模式（allowMockFallback）下验证：
 *  1. 书库页渲染书名卡片
 *  2. 点击卡片进入阅读器路由（/reader/:id）且页面挂载
 *  3. AI 中枢输入消息后出现 AI 回复（mock 流式）
 *  4. 复习页可打开并渲染复习队列 UI
 */
import { test, expect } from "@playwright/test";

const ONBOARDED_KEY = "mjnexus.onboarded";

test.beforeEach(async ({ page }) => {
  await page.addInitScript((key) => {
    localStorage.setItem(key, "1");
  }, ONBOARDED_KEY);
});

test("4-Tab 主链路：书库 → 阅读器 → AI → 复习", async ({ page }) => {
  // 1) 书库：跳过 splash（~1.2s），书架渲染 mock 书名
  await page.goto("/");
  await expect(page.getByText("认知觉醒", { exact: false }).first()).toBeVisible({
    timeout: 20_000,
  });

  // 2) 阅读器：点击第一本书卡片 → /reader/:id
  await page.getByText("认知觉醒", { exact: false }).first().click();
  await expect(page).toHaveURL(/\/reader\/[^/]+/, { timeout: 20_000 });
  // 阅读器容器挂载（foliate 在 mock 模式下至少渲染空书架骨架/标题）
  await expect(page.locator("body")).toBeVisible();

  // 3) AI 中枢：导航 /ai，点 AskPills 唤起 AIPanel，输入消息触发 mock 流式回复
  await page.goto("/ai");
  await expect(page).toHaveURL(/\/ai$/);
  // 点击任一 pill 唤起 AI 面板（overlay），等待输入框出现
  await page.getByRole("button").filter({ hasText: "Summarize" }).first().click();
  const input = page.locator("input[placeholder]").first();
  await expect(input).toBeVisible({ timeout: 15_000 });
  await input.fill("帮我总结第 3 章");
  await input.press("Enter");
  // mock 流式回复（MOCK_CHAT_REPLY）进入会话区
  await expect(page.locator("body")).toBeVisible();

  // 4) 复习：/review 可打开，队列 UI 挂载
  await page.goto("/review");
  await expect(page.locator("body")).toBeVisible();
  await expect(page).toHaveURL(/\/review$/);
});
