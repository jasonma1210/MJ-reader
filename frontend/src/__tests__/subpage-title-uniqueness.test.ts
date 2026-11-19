/**
 * 二级页标题唯一性契约测试（2026-11-19 用户反馈「标题和返回按钮名称一致」）。
 * 防止回归：一个二级页只能有一个标题源——要么 Shell 的 SubBackHeader，
 * 要么页面自带返回栏；页面内不得再出现与返回栏同名的第二个标题。
 */
import { describe, it, expect } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const SRC_DIR = fileURLToPath(new URL("../", import.meta.url));

function src(rel: string): string {
  return readFileSync(SRC_DIR + rel, "utf-8");
}

describe("二级页标题唯一性", () => {
  it("Shell 不再为 /import 渲染返回栏（ImportPage 自带，避免双重标题/双重返回键）", () => {
    const shell = src("components/shell/Shell.tsx");
    const start = shell.indexOf("const SUB_TITLE");
    const subTitleBlock = shell.slice(start, shell.indexOf("};", start));
    expect(subTitleBlock).not.toContain('"/import"');
    expect(subTitleBlock).toContain('"/notes"');
    expect(subTitleBlock).toContain('"/review"');
  });

  it("NotesLibraryPage 不再渲染 notes.title 的重复 h1（标题由 Shell 返回栏提供）", () => {
    const page = src("routes/NotesLibraryPage.tsx");
    expect(page).not.toContain("<h1");
  });

  it("LearningPathPage 只有一个 path.title 标题源（SubBackHeader，页面内无重复 h1）", () => {
    const page = src("routes/LearningPathPage.tsx");
    expect(page).toContain('titleKey="path.title"');
    // 主列表视图不得有第二个标题 h1（详情页 PathDetail 的 h1 使用具体书名，不受此约束）
    const mainView = page.slice(0, page.indexOf("function PathDetail"));
    expect(mainView).not.toMatch(/<h1[^>]*>\s*\{t\("path\.title"\)\}/);
  });
});
