import { describe, it, expect } from "vitest";
import fs from "node:fs";
import path from "node:path";

/** i18n 完整性：zh-CN 与 en 键集合必须一致（防缺译/多译） */
function flatten(obj: Record<string, unknown>, prefix = ""): string[] {
  const keys: string[] = [];
  for (const [k, v] of Object.entries(obj)) {
    const p = prefix ? `${prefix}.${k}` : k;
    if (v && typeof v === "object") keys.push(...flatten(v as Record<string, unknown>, p));
    else keys.push(p);
  }
  return keys;
}

describe("i18n locale 完整性", () => {
  const dir = path.resolve(__dirname, "../../i18n/locales");
  const zh = JSON.parse(fs.readFileSync(path.join(dir, "zh-CN.json"), "utf-8"));
  const en = JSON.parse(fs.readFileSync(path.join(dir, "en.json"), "utf-8"));

  it("zh-CN 与 en 键集合一致", () => {
    const zhKeys = new Set(flatten(zh));
    const enKeys = new Set(flatten(en));
    const missingInEn = [...zhKeys].filter((k) => !enKeys.has(k));
    const extraInEn = [...enKeys].filter((k) => !zhKeys.has(k));
    expect(missingInEn, `zh 有 en 缺失：${missingInEn.join(",")}`).toEqual([]);
    expect(extraInEn, `en 有 zh 缺失：${extraInEn.join(",")}`).toEqual([]);
  });
});
