// @vitest-environment jsdom
/**
 * P1-2/P1-3：PDF 重排引擎纯函数契约测试。
 * ① 行聚类阅读顺序（y 降序、行内 x 升序）；
 * ② x-gap 双栏判定（显著 gap 分栏、无 gap 单栏）；
 * ③ outline→TocNode 映射（丢弃 level/page，递归 children）；
 * ④ HTML 转义（& < >，防 XSS）；
 * ⑤ 空文字层返回空 HTML + hasText:false（走降级路径）。
 */
import { describe, it, expect } from "vitest";
import {
  clusterLines,
  detectColumns,
  escapeHtml,
  reflowTextContentToHtml,
  type TextItem,
} from "../renderer/pdf/reflow/pdfReflow";
import { toTocNodes } from "../renderer/pdf/reflow/reflowTypes";

const mk = (str: string, x: number, y: number): TextItem => ({
  str,
  x,
  y,
  width: 8,
  height: 12,
  fontName: "sans-serif",
});

describe("P1-2 行聚类阅读顺序", () => {
  it("①行按 y 降序（上→下），行内按 x 升序（左→右）", () => {
    // 顶部一行：B 在右、A 在左；底部一行：C
    const lines = clusterLines(
      [mk("B", 100, 800), mk("A", 10, 801), mk("C", 10, 700)],
      12,
    );
    expect(lines).toHaveLength(2);
    expect(lines[0].y).toBeGreaterThan(lines[1].y);
    expect(lines[0].items.map((i) => i.str)).toEqual(["A", "B"]);
    expect(lines[1].items.map((i) => i.str)).toEqual(["C"]);
  });

  it("①y 容差内归为同一行（tolerance = fontSize*0.5）", () => {
    const lines = clusterLines([mk("X", 10, 500), mk("Y", 40, 504)], 12);
    expect(lines).toHaveLength(1);
    expect(lines[0].items.map((i) => i.str)).toEqual(["X", "Y"]);
  });
});

describe("P1-3 x-gap 多栏判定", () => {
  it("②显著 x-gap 判定双栏，左栏在前右栏在后", () => {
    const items: TextItem[] = [];
    // 左栏 6 项：x=10..50；右栏 6 项：x=210..250（gap≈160 > 平均字宽 8 的 5 倍）
    for (let i = 0; i < 6; i++) {
      items.push(mk(`L${i}`, 10 + i * 8, 800 - i * 20));
      items.push(mk(`R${i}`, 210 + i * 8, 800 - i * 20));
    }
    const cols = detectColumns(items, "auto", 12);
    expect(cols).toHaveLength(2);
    expect(cols[0].every((i) => i.x < 130)).toBe(true);
    expect(cols[1].every((i) => i.x >= 130)).toBe(true);
    expect(cols[0]).toHaveLength(6);
    expect(cols[1]).toHaveLength(6);
  });

  it("②无显著 gap 时保持单栏", () => {
    const items = Array.from({ length: 10 }, (_, i) => mk(String(i), 10 + i * 8, 700));
    expect(detectColumns(items, "auto", 12)).toHaveLength(1);
  });

  it("②显式指定 columnCount=2 时等宽分栏", () => {
    const items = Array.from({ length: 8 }, (_, i) => mk(String(i), 10 + i * 10, 700));
    const cols = detectColumns(items, 2, 12);
    expect(cols).toHaveLength(2);
    expect(cols[0].length + cols[1].length).toBe(8);
  });
});

describe("P1-4 outline→TocNode 映射", () => {
  it("③丢弃 level/page，仅保留 title 与递归 children", () => {
    const nodes = toTocNodes([
      {
        title: "第一章",
        level: 0,
        page: 12,
        children: [
          { title: "1.1 引言", level: 1, page: 13 },
          { title: "1.2 方法", level: 1, page: 15, children: [] },
        ],
      },
      { title: "第二章", level: 0 },
    ]);
    expect(nodes).toEqual([
      {
        title: "第一章",
        children: [{ title: "1.1 引言" }, { title: "1.2 方法" }],
      },
      { title: "第二章" },
    ]);
    expect(JSON.stringify(nodes)).not.toContain("level");
    expect(JSON.stringify(nodes)).not.toContain("page");
  });
});

describe("P1-2 HTML 转义", () => {
  it("④转义 & < >，保留 XSS 防线", () => {
    expect(escapeHtml(`<script>alert("x")</script>&`)).toBe(
      `&lt;script&gt;alert("x")&lt;/script&gt;&amp;`,
    );
    const { html } = reflowTextContentToHtml({
      items: [
        {
          str: "<b>x</b> & y",
          transform: [12, 0, 0, 12, 10, 800],
          width: 80,
          height: 12,
          fontName: "sans-serif",
        },
      ],
    });
    expect(html).toContain("&lt;b&gt;x&lt;/b&gt; &amp; y");
    expect(html).not.toContain("<b>");
  });
});

describe("P1-5 空文字层降级", () => {
  it("⑤空 items 返回空 HTML 且 hasText=false", () => {
    expect(reflowTextContentToHtml({ items: [] })).toEqual({
      html: "",
      hasText: false,
    });
  });

  it("⑤仅空白字符 / 仅 marked content 视为无文字层", () => {
    expect(
      reflowTextContentToHtml({
        items: [
          { str: "   ", transform: [12, 0, 0, 12, 10, 800], width: 8, height: 12, fontName: "" },
          { type: "BeginMarkedContent" },
        ],
      }),
    ).toEqual({ html: "", hasText: false });
  });

  it("⑤有文字层时 hasText=true 且输出 <p> 段落", () => {
    const { html, hasText } = reflowTextContentToHtml({
      items: [
        {
          str: "有文字层",
          transform: [12, 0, 0, 12, 10, 800],
          width: 48,
          height: 12,
          fontName: "sans-serif",
        },
      ],
    });
    expect(hasText).toBe(true);
    expect(html).toBe("<p>有文字层</p>");
  });
});
