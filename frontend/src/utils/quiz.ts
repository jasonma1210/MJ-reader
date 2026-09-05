/** 正确选项字母 → 选项索引（对齐后端：选择题 answer 存标准答案字母 A-F） */
export function parseCorrectIndex(answer: string | null | undefined): number {
  const letter = (answer ?? "").trim().toUpperCase();
  const m = /^[（(]?([A-F])[)）]?/.exec(letter);
  if (!m) return -1;
  const labels = ["A", "B", "C", "D", "E", "F"];
  return labels.indexOf(m[1]);
}
