import { describe, expect, it } from "vitest";
import {
  resolveTimeBand,
  resolvePlatformConfig,
  resolveProfile,
  resolveReaderColors,
  type ResolveProfileInput,
} from "../renderer/visual/einkProfile";
import {
  MODE_TOKENS,
  TEMP_BANDS,
  GREYSCALE_HIGHLIGHTS,
  GREYSCALE_HIGHLIGHT_FG,
} from "../renderer/visual/visionTokens";

const at = (h: number, m = 0) => new Date(2026, 0, 1, h, m);

const BASE: ResolveProfileInput = {
  mode: "standard",
  bgColor: "#F7F6F2",
  textColor: "#1A1A1A",
  colorTempFollow: true,
  warmth: 0,
  intensity: 0,
  band: "day",
};

describe("resolveTimeBand：三段 07:00 / 17:00 / 21:00", () => {
  // 取证说明：实现按设计文档 §6.1 代码 `h>=21 || h<7 -> night`，故 06:59 仍属夜间段。
  // 任务单中「06:59→day」与该代码 / §5.3「夜间 21:00–07:00」冲突，已在交付报告标注。
  it("覆盖 7 个边界点（06:59 / 07:00 / 16:59 / 17:00 / 20:59 / 21:00 / 03:00）", () => {
    expect(resolveTimeBand(at(6, 59))).toBe("night");
    expect(resolveTimeBand(at(7, 0))).toBe("day");
    expect(resolveTimeBand(at(16, 59))).toBe("day");
    expect(resolveTimeBand(at(17, 0))).toBe("dusk");
    expect(resolveTimeBand(at(20, 59))).toBe("dusk");
    expect(resolveTimeBand(at(21, 0))).toBe("night");
    expect(resolveTimeBand(at(3, 0))).toBe("night");
  });

  it("每段内部任意时刻归类稳定", () => {
    expect(resolveTimeBand(at(12, 30))).toBe("day");
    expect(resolveTimeBand(at(19, 0))).toBe("dusk");
    expect(resolveTimeBand(at(23, 59))).toBe("night");
  });
});

describe("resolvePlatformConfig：平台 + 模式配置", () => {
  it("桌面 standard 默认值不变（prefetch 2 / 并发 1 / dpr 1.5）", () => {
    expect(resolvePlatformConfig(false, "standard")).toEqual({
      prefetchRange: 2,
      prefetchConcurrency: 1,
      dprCap: 1.5,
    });
  });

  it("Tauri standard 并发提升到 2，预取与 dpr 不变", () => {
    expect(resolvePlatformConfig(true, "standard")).toEqual({
      prefetchRange: 2,
      prefetchConcurrency: 2,
      dprCap: 1.5,
    });
  });

  it("ink 档预取 1 且 dpr 上限 1", () => {
    expect(resolvePlatformConfig(false, "ink")).toEqual({
      prefetchRange: 1,
      prefetchConcurrency: 1,
      dprCap: 1,
    });
    expect(resolvePlatformConfig(true, "ink")).toEqual({
      prefetchRange: 1,
      prefetchConcurrency: 2,
      dprCap: 1,
    });
  });
});

describe("resolveProfile：三态参数（§5.2）", () => {
  it("standard：背景跟随 readerStore、前景 #1A1A1A、filter none、不禁动画", () => {
    const p = resolveProfile({ ...BASE, mode: "standard" });
    expect(p.background).toBe("#F7F6F2");
    expect(p.foreground).toBe("#1A1A1A");
    expect(p.modeFilter).toBe("none");
    expect(p.reduceMotion).toBe(false);
    expect(p.pdfFilter).toBeNull(); // intensity=0 → 无色温滤镜
  });

  it("simulated：#EFEDE6 / #1F1F1F / saturate+contrast / 禁动画", () => {
    const p = resolveProfile({ ...BASE, mode: "simulated" });
    expect(p.background).toBe("#EFEDE6");
    expect(p.foreground).toBe("#1F1F1F");
    expect(p.modeFilter).toBe("saturate(0.55) contrast(1.05)");
    expect(p.reduceMotion).toBe(true);
    expect(p.pdfFilter).toContain("saturate(0.55)");
  });

  it("ink：#FFFFFF / #000000 / grayscale+contrast / 禁动画", () => {
    const p = resolveProfile({ ...BASE, mode: "ink" });
    expect(p.background).toBe("#FFFFFF");
    expect(p.foreground).toBe("#000000");
    expect(p.modeFilter).toBe("grayscale(1) contrast(1.15)");
    expect(p.reduceMotion).toBe(true);
    expect(p.pdfFilter).toContain("grayscale(1)");
  });

  it("灰度高亮 5 级令牌固定", () => {
    expect([...GREYSCALE_HIGHLIGHTS]).toEqual([
      "#E0E0E0",
      "#C7C7C7",
      "#A8A8A8",
      "#8A8A8A",
      "#6B6B6B",
    ]);
    expect([...GREYSCALE_HIGHLIGHT_FG]).toEqual([
      "#000000",
      "#000000",
      "#000000",
      "#FFFFFF",
      "#FFFFFF",
    ]);
  });
});

describe("软件色温三段换算（§5.3）", () => {
  it("自动日间 intensity=100 → sepia(0.04) hue-rotate(-6deg)，无遮罩", () => {
    const p = resolveProfile({ ...BASE, intensity: 100, band: "day" });
    expect(p.warmth).toBe(-30);
    expect(p.tempFilter).toBe("sepia(0.04) hue-rotate(-6deg)");
    expect(p.overlayColor).toBeNull();
  });

  it("自动黄昏 intensity=100 → sepia(0.14) hue-rotate(-4deg)", () => {
    const p = resolveProfile({ ...BASE, intensity: 100, band: "dusk" });
    expect(p.warmth).toBe(40);
    expect(p.tempFilter).toBe("sepia(0.14) hue-rotate(-4deg)");
  });

  it("自动夜间 intensity=100 → brightness(0.86) + 0.22 遮罩", () => {
    const p = resolveProfile({ ...BASE, intensity: 100, band: "night" });
    expect(p.warmth).toBe(70);
    expect(p.tempFilter).toBe("sepia(0.22) brightness(0.86)");
    expect(p.overlayColor).toBe("rgba(0, 0, 0, 0.22)");
  });

  it("强度按比例缩放：intensity=50 时 sepia 减半、遮罩减半", () => {
    const p = resolveProfile({ ...BASE, intensity: 50, band: "day" });
    expect(p.tempFilter).toBe("sepia(0.02) hue-rotate(-3deg)");
    const night = resolveProfile({ ...BASE, intensity: 50, band: "night" });
    expect(night.overlayColor).toBe("rgba(0, 0, 0, 0.11)");
  });

  it("手动 warmth 生效且与自动值不同", () => {
    const warm = resolveProfile({
      ...BASE,
      colorTempFollow: false,
      warmth: 100,
      intensity: 100,
      band: "day",
    });
    const cool = resolveProfile({
      ...BASE,
      colorTempFollow: false,
      warmth: -100,
      intensity: 100,
      band: "day",
    });
    expect(warm.warmth).toBe(100);
    expect(cool.warmth).toBe(-100);
    expect(warm.tempFilter).not.toBe(cool.tempFilter);
  });
});

describe("令牌表与 readerStore 颜色解析", () => {
  it("三段自动色温值固定 -30 / +40 / +70", () => {
    expect(TEMP_BANDS.day.autoWarmth).toBe(-30);
    expect(TEMP_BANDS.dusk.autoWarmth).toBe(40);
    expect(TEMP_BANDS.night.autoWarmth).toBe(70);
  });

  it("模式令牌表包含三态", () => {
    expect(Object.keys(MODE_TOKENS)).toEqual(["standard", "simulated", "ink"]);
  });

  it("readerStore key 解析出配套前景，未知背景 key 回退首个预设", () => {
    expect(resolveReaderColors("dark", "default")).toEqual({
      bgColor: "#101418",
      textColor: "#1a1a1a",
    });
    expect(resolveReaderColors("unknown-bg", "default").bgColor).toBe("#ffffff");
  });
});
