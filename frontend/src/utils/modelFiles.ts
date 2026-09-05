/**
 * 模型文件纯逻辑工具：量化标签解析 + 文件大小人性化显示。
 * 与后端 model_hub.rs 的 parse_quant 语义对齐（前端仅作展示兜底），
 * 无副作用、无依赖，便于 vitest 单测。
 */

/**
 * 从 GGUF 文件名解析量化标签。
 * 覆盖 llama.cpp 常见量化族：Q4_K_M / Q5_K_S / Q8_0 / IQ4_XS / Q2_K / Q6_K，
 * 以及非量化精度 F16 / BF16 / F32 / FP8。
 *
 * 示例：
 * - "Qwen3-1.7B-Q4_K_M.gguf" → "Q4_K_M"
 * - "Llama-3.2-1B-Instruct-Q8_0.gguf" → "Q8_0"
 * - "mmproj-Qwen3-1.7B.gguf" → null（投影文件无量化）
 */
export function parseQuantLabel(fileName: string): string | null {
  if (!fileName) return null;
  const base = fileName.toLowerCase().endsWith(".gguf")
    ? fileName.slice(0, -".gguf".length)
    : fileName;
  // 量化 token 必须由行首或分隔符（- _ .）界定，避免误吞模型名中的字母数字
  const match = base.match(
    /(?:^|[-_.])(iq\d+(?:_[a-z0-9]+)*|q\d+(?:_[a-z0-9]+)*|f16|bf16|f32|fp8)(?=$|[-_.])/i,
  );
  return match ? match[1].toUpperCase() : null;
}

/** 是否为多模态投影文件（mmproj 前缀） */
export function isProjectorFile(fileName: string): boolean {
  const lower = fileName.toLowerCase();
  return lower.startsWith("mmproj") || lower.includes(".mmproj.");
}

/** 是否分片 GGUF（形如 xxx-00001-of-00003.gguf，需整组下载，排序沉底） */
export function isShardedGguf(fileName: string): boolean {
  return /-\d{5}-of-\d{5}\.gguf$/i.test(fileName);
}

/** 4bit 推荐优先级：Q4_K_M 为社区公认质量/体积均衡首选（2026-09-04 用户裁定默认推荐 4bit） */
const QUANT_4BIT_PRIORITY = ["Q4_K_M", "Q4_K_S", "IQ4_XS", "IQ4_NL", "Q4_0"];

/** 弹层文件条目所需最小字段（便于单测复用） */
export interface QuantVariantLike {
  fileKind: string;
  fileName: string;
  quant: string | null;
  sizeBytes: number;
}

/**
 * 从文件清单中选出推荐下载项：按 4bit 优先级取第一个命中的非分片 GGUF。
 * 仓库无 4bit 变体时返回 null（不强推其他量化）。
 */
export function pickRecommendedGguf<T extends QuantVariantLike>(files: T[]): T | null {
  const gguf = files.filter((f) => f.fileKind === "gguf" && !isShardedGguf(f.fileName));
  for (const q of QUANT_4BIT_PRIORITY) {
    const hit = gguf.find((f) => (f.quant ?? "").toUpperCase() === q);
    if (hit) return hit;
  }
  return null;
}

/** 弹层文件排序档位：推荐 4bit → 投影 → 其他 4bit GGUF → 其他量化 GGUF → 分片 GGUF */
function sortRank<T extends QuantVariantLike>(f: T, recommended: T | null): number {
  if (recommended !== null && f.fileName === recommended.fileName) return 0;
  if (f.fileKind === "projector") return 1;
  if (f.fileKind === "gguf") {
    if (isShardedGguf(f.fileName)) return 4;
    const q = (f.quant ?? "").toUpperCase();
    return q.startsWith("Q4") || q.startsWith("IQ4") ? 2 : 3;
  }
  return 3;
}

/** 文件清单展示排序：推荐 4bit 置顶，其余按档位 + 体积升序（2026-09-04） */
export function sortModelFiles<T extends QuantVariantLike>(files: T[]): T[] {
  const recommended = pickRecommendedGguf(files);
  return [...files].sort((a, b) => {
    const ra = sortRank(a, recommended);
    const rb = sortRank(b, recommended);
    if (ra !== rb) return ra - rb;
    return a.sizeBytes - b.sizeBytes;
  });
}

/**
 * 文件大小人性化显示：B → KB → MB → GB → TB。
 * 保留 1 位小数；整百以上或原始字节值取整显示。
 */
export function formatFileSize(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  const text = value >= 10 || unit === 0 ? value.toFixed(0) : value.toFixed(1);
  return `${text} ${units[unit]}`;
}
