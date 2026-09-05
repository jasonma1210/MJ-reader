import { describe, it, expect } from "vitest";
import { unsupportedReason, rawWeightsHint, readmeIntroText } from "../modelSupport";

describe("unsupportedReason", () => {
  const card = (repoId: string, tags: string[] = []) => ({
    repoId,
    name: repoId.split("/").pop() ?? repoId,
    tags,
  });

  it("MLX 仓库：无 MLX 运行时的设备不支持", () => {
    expect(unsupportedReason(card("mlx-community/Qwen3-1.7B-4bit"), false)).toBe("mlx");
  });

  it("MLX 仓库：macOS（有 mlx-lm）支持", () => {
    expect(unsupportedReason(card("mlx-community/Qwen3-1.7B-4bit"), true)).toBeNull();
  });

  it("AWQ / GPTQ / EXL2 / ONNX 专有格式不支持", () => {
    expect(unsupportedReason(card("baichuan-inc/Baichuan2-7B-Chat-AWQ"), false)).toBe("format");
    expect(unsupportedReason(card("Qwen/Qwen3-8B-GPTQ-Int4"), false)).toBe("format");
    expect(unsupportedReason(card("TheBloke/Llama-2-7B-Chat-EXL2"), false)).toBe("format");
    expect(unsupportedReason(card("org/model-onnx", ["onnx"]), false)).toBe("format");
  });

  it("GPTQ 等标记允许数字后缀", () => {
    expect(unsupportedReason(card("org/model-GPTQ4"), false)).toBe("format");
  });

  it("GGUF 仓库支持（即使标签含格式词也不误伤）", () => {
    expect(unsupportedReason(card("Qwen/Qwen3-1.7B-GGUF"), false)).toBeNull();
    expect(unsupportedReason(card("org/model-GGUF", ["awq"]), false)).toBeNull();
  });

  it("FP8 等非 GGUF 精度标记判为不支持格式", () => {
    expect(unsupportedReason(card("Qwen/Qwen3-4B-FP8"), false)).toBe("format");
  });

  it("普通原始权重仓不误判（支持性由文件弹层/加载兜底）", () => {
    expect(unsupportedReason(card("Qwen/Qwen3-1.7B"), false)).toBeNull();
  });
});

describe("rawWeightsHint", () => {
  const card = (repoId: string) => ({
    repoId,
    name: repoId.split("/").pop() ?? repoId,
    tags: [],
  });

  it("原始权重仓命中提示（无 GGUF/MLX/量化标记）", () => {
    expect(rawWeightsHint(card("Qwen/Qwen3-4B"))).toBe(true);
    expect(rawWeightsHint(card("meta-llama/Llama-3.2-3B-Instruct"))).toBe(true);
    expect(rawWeightsHint(card("Qwen/Qwen3.5-4B"))).toBe(true);
  });

  it("GGUF / MLX / 量化命名 / 专有格式仓不提示", () => {
    expect(rawWeightsHint(card("Qwen/Qwen3-4B-GGUF"))).toBe(false);
    expect(rawWeightsHint(card("mlx-community/Qwen3-1.7B-4bit"))).toBe(false);
    expect(rawWeightsHint(card("org/model-Q4_K_M"))).toBe(false);
    expect(rawWeightsHint(card("Qwen/Qwen3-4B-FP8"))).toBe(false);
  });

  it("不误吞 Qwen 等品牌名中的字母数字", () => {
    // "Qwen3" 中 Q 后不是数字，不构成量化标记
    expect(rawWeightsHint(card("Qwen/Qwen3-1.7B"))).toBe(true);
  });
});

describe("readmeIntroText", () => {
  it("剥离 front matter / 代码块 / 图片 / 链接 / 标题符号", () => {
    const md = [
      "---",
      "license: apache-2.0",
      "tags: [text-generation]",
      "---",
      "# Qwen3-1.7B",
      "",
      "![logo](https://example.com/logo.png)",
      "",
      "A strong [on-device](https://example.com) model.",
      "",
      "```python",
      "print('hi')",
      "```",
      "",
      "## Highlights",
      "- Fast inference",
      "- **Small** size",
    ].join("\n");
    const text = readmeIntroText(md);
    expect(text).not.toContain("license");
    expect(text).not.toContain("logo.png");
    expect(text).not.toContain("print");
    expect(text).toContain("Qwen3-1.7B");
    expect(text).toContain("on-device");
    expect(text).toContain("· Fast inference");
    expect(text).toContain("Small");
    expect(text).not.toContain("**");
  });

  it("超长文本截断并追加省略号", () => {
    const text = readmeIntroText("字".repeat(1000), 480);
    expect(text.length).toBe(481);
    expect(text.endsWith("…")).toBe(true);
  });

  it("空输入返回空串", () => {
    expect(readmeIntroText("")).toBe("");
  });
});
