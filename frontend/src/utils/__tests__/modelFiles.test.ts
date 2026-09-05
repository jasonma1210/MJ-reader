import { describe, it, expect } from "vitest";
import {
  parseQuantLabel,
  isProjectorFile,
  formatFileSize,
  isShardedGguf,
  pickRecommendedGguf,
  sortModelFiles,
  type QuantVariantLike,
} from "../modelFiles";

describe("parseQuantLabel", () => {
  it("解析常见 K-quant 标签", () => {
    expect(parseQuantLabel("Qwen3-1.7B-Q4_K_M.gguf")).toBe("Q4_K_M");
    expect(parseQuantLabel("Llama-3.2-1B-Instruct-Q5_K_S.gguf")).toBe("Q5_K_S");
    expect(parseQuantLabel("gemma-3-1b-it-Q8_0.gguf")).toBe("Q8_0");
    expect(parseQuantLabel("model-Q2_K.gguf")).toBe("Q2_K");
    expect(parseQuantLabel("model-Q6_K.gguf")).toBe("Q6_K");
  });

  it("解析 I-quant 与精度后缀", () => {
    expect(parseQuantLabel("model-IQ4_XS.gguf")).toBe("IQ4_XS");
    expect(parseQuantLabel("model-F16.gguf")).toBe("F16");
    expect(parseQuantLabel("model-bf16.gguf")).toBe("BF16");
    expect(parseQuantLabel("model-F32.gguf")).toBe("F32");
  });

  it("无量化信息时返回 null", () => {
    expect(parseQuantLabel("mmproj-Qwen3-1.7B.gguf")).toBeNull();
    expect(parseQuantLabel("Qwen3-1.7B.gguf")).toBeNull();
    expect(parseQuantLabel("")).toBeNull();
  });

  it("不误吞模型名中的字母数字", () => {
    expect(parseQuantLabel("DeepSeek-R1-Distill-Qwen-1.5B-Q4_K_M.gguf")).toBe(
      "Q4_K_M",
    );
    // 量化 token 必须由分隔符界定：模型名内嵌的 "abq8" 不算量化标签
    expect(parseQuantLabel("Qwen2.5-1.5B-Instruct.abq8.gguf")).toBeNull();
  });
});

describe("isProjectorFile", () => {
  it("识别 mmproj 前缀 / 中缀", () => {
    expect(isProjectorFile("mmproj-Qwen3-1.7B.gguf")).toBe(true);
    expect(isProjectorFile("model.mmproj.v1.gguf")).toBe(true);
    expect(isProjectorFile("Qwen3-1.7B-Q4_K_M.gguf")).toBe(false);
  });
});

describe("formatFileSize", () => {
  it("B / KB / MB / GB 阶梯", () => {
    expect(formatFileSize(0)).toBe("0 B");
    expect(formatFileSize(512)).toBe("512 B");
    expect(formatFileSize(2048)).toBe("2.0 KB");
    expect(formatFileSize(15 * 1024 * 1024)).toBe("15 MB");
    expect(formatFileSize(1.2 * 1024 * 1024 * 1024)).toBe("1.2 GB");
  });

  it("非法输入回退 0 B", () => {
    expect(formatFileSize(-1)).toBe("0 B");
    expect(formatFileSize(Number.NaN)).toBe("0 B");
  });
});

describe("isShardedGguf", () => {
  it("识别分片命名", () => {
    expect(isShardedGguf("model-00001-of-00003.gguf")).toBe(true);
    expect(isShardedGguf("MODEL-00002-OF-00004.GGUF")).toBe(true);
  });

  it("单体文件不分片", () => {
    expect(isShardedGguf("Qwen3-1.7B-Q4_K_M.gguf")).toBe(false);
    expect(isShardedGguf("model-1-of-3.gguf")).toBe(false);
  });
});

describe("pickRecommendedGguf / sortModelFiles", () => {
  const mk = (fileName: string, quant: string | null, sizeBytes: number, fileKind = "gguf"): QuantVariantLike => ({
    fileName,
    quant,
    sizeBytes,
    fileKind,
  });

  it("推荐 Q4_K_M 优先，其次 Q4_K_S", () => {
    const files = [
      mk("a-Q8_0.gguf", "Q8_0", 2000),
      mk("a-Q4_K_S.gguf", "Q4_K_S", 1100),
      mk("a-Q4_K_M.gguf", "Q4_K_M", 1200),
    ];
    expect(pickRecommendedGguf(files)?.quant).toBe("Q4_K_M");
  });

  it("无 4bit 变体时不强推", () => {
    const files = [mk("a-Q8_0.gguf", "Q8_0", 2000)];
    expect(pickRecommendedGguf(files)).toBeNull();
  });

  it("排序：推荐 4bit 置顶 → 投影 → 其他 4bit → 其他量化 → 分片", () => {
    const files = [
      mk("m-00001-of-00002.gguf", "Q4_K_M", 900),
      mk("a-F16.gguf", "F16", 4000),
      mk("b-IQ4_XS.gguf", "IQ4_XS", 1000),
      mk("mmproj-model.gguf", null, 500, "projector"),
      mk("m-Q4_K_M.gguf", "Q4_K_M", 1200),
    ];
    const sorted = sortModelFiles(files);
    expect(sorted[0].quant).toBe("Q4_K_M");
    expect(sorted[0].fileName).toBe("m-Q4_K_M.gguf");
    expect(sorted[1].fileKind).toBe("projector");
    expect(sorted[2].quant).toBe("IQ4_XS");
    expect(sorted[3].quant).toBe("F16");
    expect(sorted[4].fileName).toBe("m-00001-of-00002.gguf");
  });

  it("同档位按体积升序", () => {
    const files = [mk("big-Q8_0.gguf", "Q8_0", 3000), mk("small-Q8_0.gguf", "Q8_0", 1500)];
    const sorted = sortModelFiles(files);
    expect(sorted[0].fileName).toBe("small-Q8_0.gguf");
  });
});
