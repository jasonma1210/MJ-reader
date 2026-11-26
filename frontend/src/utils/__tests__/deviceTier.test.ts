import { describe, it, expect } from "vitest";
import {
  resolveDeviceLimits,
  resolveDeviceTier,
  type DeviceSignals,
} from "../deviceTier";

describe("resolveDeviceTier", () => {
  it("按真实内存分档（桌面口径不变）", () => {
    expect(resolveDeviceTier({ deviceMemoryGb: 2 })).toBe("low");
    expect(resolveDeviceTier({ deviceMemoryGb: 4 })).toBe("mid");
    expect(resolveDeviceTier({ deviceMemoryGb: 8 })).toBe("high");
    expect(resolveDeviceTier({ deviceMemoryGb: 12 })).toBe("high");
  });

  it("移动端封顶：8GB Android 降为 mid，不做 high（WebView 堆仅 256MB）", () => {
    const android: DeviceSignals = { isAndroid: true, isMobile: true };
    expect(resolveDeviceTier({ ...android, deviceMemoryGb: 4 })).toBe("low");
    expect(resolveDeviceTier({ ...android, deviceMemoryGb: 8 })).toBe("mid");
    expect(resolveDeviceTier({ ...android, deviceMemoryGb: 12 })).toBe("high");
    const ios: DeviceSignals = { isIOS: true, isMobile: true };
    expect(resolveDeviceTier({ ...ios, deviceMemoryGb: 8 })).toBe("mid");
  });

  it("移动端内存不可得时按核心数也降一档", () => {
    const android: DeviceSignals = { isAndroid: true, deviceMemoryGb: null };
    expect(resolveDeviceTier({ ...android, hardwareConcurrency: 4 })).toBe("low");
    expect(resolveDeviceTier({ ...android, hardwareConcurrency: 6 })).toBe("mid");
    expect(resolveDeviceTier({ ...android, hardwareConcurrency: 8 })).toBe("high");
  });

  it("内存不可得时按核心数分档（桌面口径）", () => {
    const base: DeviceSignals = { deviceMemoryGb: null };
    expect(resolveDeviceTier({ ...base, hardwareConcurrency: 2 })).toBe("low");
    expect(resolveDeviceTier({ ...base, hardwareConcurrency: 4 })).toBe("mid");
    expect(resolveDeviceTier({ ...base, hardwareConcurrency: 6 })).toBe("high");
  });

  it("信号全缺失时保守取中档", () => {
    expect(resolveDeviceTier({})).toBe("mid");
  });

  it("无效内存值不参与判定", () => {
    expect(resolveDeviceTier({ deviceMemoryGb: Number.NaN, hardwareConcurrency: 8 })).toBe("high");
  });
});

describe("resolveDeviceLimits", () => {
  it("低端机收紧 PDF、封面、TTS 与媒体上限", () => {
    const low = resolveDeviceLimits({ deviceMemoryGb: 2 });
    const high = resolveDeviceLimits({ deviceMemoryGb: 8 });
    expect(low.pdfPageLimit).toBeLessThan(high.pdfPageLimit);
    expect(low.coverCacheLimit).toBeLessThan(high.coverCacheLimit);
    expect(low.ttsCacheLimit).toBeLessThan(high.ttsCacheLimit);
    expect(low.mediaQuotaMb).toBeLessThan(high.mediaQuotaMb);
    expect(low.ioChunkMb).toBe(high.ioChunkMb);
  });
});
