/**
 * 搜索结果「本系统是否支持」纯前端启发式判定（2026-09-04）。
 *
 * 本系统端侧引擎为 llamacpp（GGUF 专用）：
 * - MLX 权重仅 macOS（mlx-lm）可运行，iPhone/Android 不支持；
 * - AWQ / GPTQ / EXL2 / ONNX 等专有量化/导出格式无法被 llama.cpp 加载；
 * - 架构级不支持（该版 llama.cpp 未收录的新架构）无法在搜索期零成本判定，
 *   由「加载 / 测试」阶段后端真实报错兜底（GGUF 头校验 + 架构归因消息）。
 *
 * 纯函数、无副作用、无 UI 文案（i18n 棘轮），便于 vitest 单测。
 */

/** llama.cpp 无法加载的专有量化 / 导出格式标记（允许数字后缀，如 GPTQ4） */
const UNSUPPORTED_FORMAT_RE = /\b(AWQ|GPTQ|EXL[0-9]?|ONNX|OPENVINO|TENSORRT|FP8)[0-9]*\b/;

/** 量化标记特征（文件名/仓库名中带 Q4_K_M / IQ4_XS / FP8 等 → 大概率是 GGUF/量化仓库） */
const QUANT_LIKE_RE = /(?:^|[^A-Z0-9])(I?Q[0-9]|F16|BF16|FP8|MXFP4)(?:$|[^A-Z0-9])/;

export type UnsupportedKind = "mlx" | "format";

/** 判定输入（ModelCard 的最小字段面，便于单测） */
export interface SupportCheckCard {
  repoId: string;
  name: string;
  tags: string[];
}

/**
 * 返回不支持原因；null = 启发式判定为支持（可进入文件弹层）。
 * - "mlx"：MLX 权重仓库且当前设备无 MLX 运行时；
 * - "format"：仓库名/标签命中 AWQ/GPTQ 等格式且未标 GGUF。
 */
export function unsupportedReason(
  card: SupportCheckCard,
  canRunMlx: boolean,
): UnsupportedKind | null {
  const hay = `${card.repoId} ${card.name} ${card.tags.join(" ")}`.toUpperCase();
  if (hay.includes("MLX") && !canRunMlx) return "mlx";
  if (!hay.includes("GGUF") && UNSUPPORTED_FORMAT_RE.test(hay)) return "format";
  return null;
}

/**
 * 原始权重仓提示（信息性，非禁用）：仓库名/模型名既无 GGUF/MLX 标记、
 * 也无量化标记与专有格式标记 → 大概率只有 safetensors 原始权重。
 * 搜索结果打「原始权重」徽章引导用户选 GGUF 仓库；仍可点入
 * （弹层内后端会自动探测同名 -GGUF 兄弟仓库，命中即展示量化文件）。
 */
export function rawWeightsHint(card: SupportCheckCard): boolean {
  const hay = `${card.repoId} ${card.name}`.toUpperCase();
  if (hay.includes("GGUF") || hay.includes("MLX")) return false;
  if (UNSUPPORTED_FORMAT_RE.test(hay)) return false;
  return !QUANT_LIKE_RE.test(hay);
}

/**
 * README markdown → 纯文本简介：剥离 YAML front matter / 代码块 / 图片 / HTML /
 * 链接 / 标题与强调符号，压缩空白，截断至 maxChars（移动端渲染保护）。
 */
export function readmeIntroText(markdown: string, maxChars = 480): string {
  let text = markdown;
  text = text.replace(/^---[\s\S]*?---/, " "); // YAML front matter
  text = text.replace(/```[\s\S]*?```/g, " "); // 代码块
  text = text.replace(/<[^>\n]+>/g, " "); // HTML 标签
  text = text.replace(/!\[[^\]]*\]\([^)]*\)/g, " "); // 图片
  text = text.replace(/\[([^\]]*)\]\([^)]*\)/g, "$1"); // 链接保留文字
  text = text.replace(/^[ \t]{0,3}#{1,6}[ \t]+/gm, ""); // 标题符号
  text = text.replace(/^[ \t]{0,3}[-*_]{3,}[ \t]*$/gm, " "); // 分隔线
  text = text.replace(/^[ \t]{0,3}>[ \t]?/gm, ""); // 引用
  text = text.replace(/[*_`~]/g, ""); // 强调符号
  text = text.replace(/^[ \t]*[-+*][ \t]+/gm, "· "); // 列表符号
  text = text.replace(/[ \t]*\n+[ \t]*/g, "\n");
  text = text.trim();
  if (text.length > maxChars) {
    // 按 char 截断，避免切开多字节字符（slice 按 UTF-16 code unit，中文 BMP 内安全）
    text = `${text.slice(0, maxChars)}…`;
  }
  return text;
}
