/**
 * 设备资源档位（1.0 工程底座）
 *
 * 把内存、性能、存储相关的上限集中到一处，避免各模块各自硬编码。
 * 规则为纯函数、显式判定，便于单测与跨端一致；不读取用户隐私数据。
 */
import { isAndroid, isIOS } from "./platform";

export type DeviceTier = "low" | "mid" | "high";

export interface DeviceResourceLimits {
  tier: DeviceTier;
  /** PDF 页面缓存硬上限（页数） */
  pdfPageLimit: number;
  /** 封面 data URI LRU 容量（张） */
  coverCacheLimit: number;
  /** TTS 句子级音频缓存容量（条） */
  ttsCacheLimit: number;
  /** note_media 容量阈值（MB） */
  mediaQuotaMb: number;
  /** 备份 / 大文件哈希分块大小（MB） */
  ioChunkMb: number;
}

export interface DeviceSignals {
  /** navigator.deviceMemory，单位 GB；Chrome/Android 可用，WebKit 通常为 undefined */
  deviceMemoryGb?: number | null;
  /** navigator.hardwareConcurrency，逻辑核心数 */
  hardwareConcurrency?: number | null;
  isMobile?: boolean;
  isIOS?: boolean;
  isAndroid?: boolean;
}

const LIMITS: Record<DeviceTier, DeviceResourceLimits> = {
  low: {
    tier: "low",
    pdfPageLimit: 6,
    coverCacheLimit: 30,
    ttsCacheLimit: 8,
    mediaQuotaMb: 256,
    ioChunkMb: 8,
  },
  mid: {
    tier: "mid",
    pdfPageLimit: 12,
    coverCacheLimit: 60,
    ttsCacheLimit: 24,
    mediaQuotaMb: 768,
    ioChunkMb: 8,
  },
  high: {
    tier: "high",
    pdfPageLimit: 20,
    coverCacheLimit: 120,
    ttsCacheLimit: 64,
    mediaQuotaMb: 2048,
    ioChunkMb: 8,
  },
};

/** 显式规则：优先真实内存档位；内存不可得时退到核心数；两者都不可得按中档。 */
export function resolveDeviceTier(signals: DeviceSignals): DeviceTier {
  const memory = signals.deviceMemoryGb;
  if (typeof memory === "number" && Number.isFinite(memory)) {
    if (memory <= 2) return "low";
    if (memory <= 4) return "mid";
    return "high";
  }
  const cores = signals.hardwareConcurrency;
  if (typeof cores === "number" && Number.isFinite(cores)) {
    if (cores <= 2) return "low";
    if (cores <= 4) return "mid";
    return "high";
  }
  return "mid";
}

export function resolveDeviceLimits(signals: DeviceSignals): DeviceResourceLimits {
  return LIMITS[resolveDeviceTier(signals)];
}

/** 在 WebView 中读取可用信号；SSR / Node 下返回空对象走中档。 */
export function currentDeviceSignals(): DeviceSignals {
  if (typeof navigator === "undefined") return {};
  const nav = navigator as Navigator & { deviceMemory?: number };
  return {
    deviceMemoryGb: nav.deviceMemory ?? null,
    hardwareConcurrency: nav.hardwareConcurrency ?? null,
    isMobile: isIOS() || isAndroid(),
    isIOS: isIOS(),
    isAndroid: isAndroid(),
  };
}

let cachedLimits: DeviceResourceLimits | null = null;

/** 运行时唯一真源：信号在 App 生命周期内不变，取一次即可。 */
export function getDeviceLimits(): DeviceResourceLimits {
  if (!cachedLimits) cachedLimits = resolveDeviceLimits(currentDeviceSignals());
  return cachedLimits;
}
