import { describe, it, expect } from "vitest";
import {
  resolveDeviceLimits,
  resolveDeviceTier,
  type DeviceSignals,
} from "../deviceTier";

describe("resolveDeviceTier", () => {
  it("按真实内存分档", () => {
    expect(resolveDeviceTier({ deviceMemoryGb: 2 })).toBe("low");
    expect(resolveDeviceTier({ deviceMemoryGb: 4 })).toBe("mid");
    expect(resolveDeviceTier({ deviceMemoryGb: 8 })).toBe("high");
    expect(resolveDeviceTier({ deviceMemoryGb: 12 })).toBe("high");
  });

  it("内存不可得时按核心数分档", () => {
    const base: DeviceSignals = { deviceMemoryGb: null, isIOS: true };
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
