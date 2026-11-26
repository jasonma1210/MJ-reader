/**
 * 书籍格式书签标签（2026-11-26）：
 * 统一格式值 → 展示文案 / 配色，供网格卡片与列表行共用。
 * 颜色为固定色值（书签隐喻），浅色/深色主题下均保持可辨识。
 */

export interface FormatBadge {
  label: string;
  /** 书签底色 */
  background: string;
  /** 文字色 */
  color: string;
}

/** 按扩展名归类的配色；兜底用中性灰。 */
const PALETTE: Record<string, FormatBadge> = {
  epub: { label: "EPUB", background: "#3b82f6", color: "#ffffff" },
  pdf: { label: "PDF", background: "#ef4444", color: "#ffffff" },
  mobi: { label: "MOBI", background: "#8b5cf6", color: "#ffffff" },
  azw: { label: "AZW", background: "#7c3aed", color: "#ffffff" },
  azw3: { label: "AZW3", background: "#6d28d9", color: "#ffffff" },
  cbz: { label: "CBZ", background: "#f59e0b", color: "#1f1300" },
  zip: { label: "ZIP", background: "#d97706", color: "#ffffff" },
  txt: { label: "TXT", background: "#10b981", color: "#ffffff" },
  md: { label: "MD", background: "#059669", color: "#ffffff" },
  html: { label: "HTML", background: "#0ea5e9", color: "#ffffff" },
  xhtml: { label: "XHTML", background: "#0284c7", color: "#ffffff" },
  xml: { label: "XML", background: "#0369a1", color: "#ffffff" },
  mhtml: { label: "MHTML", background: "#075985", color: "#ffffff" },
  fb2: { label: "FB2", background: "#14b8a6", color: "#ffffff" },
  docx: { label: "DOCX", background: "#2563eb", color: "#ffffff" },
  doc: { label: "DOC", background: "#1d4ed8", color: "#ffffff" },
  pptx: { label: "PPTX", background: "#ea580c", color: "#ffffff" },
  ppt: { label: "PPT", background: "#c2410c", color: "#ffffff" },
  xlsx: { label: "XLSX", background: "#16a34a", color: "#ffffff" },
  xls: { label: "XLS", background: "#15803d", color: "#ffffff" },
  rtf: { label: "RTF", background: "#64748b", color: "#ffffff" },
  odt: { label: "ODT", background: "#475569", color: "#ffffff" },
  ods: { label: "ODS", background: "#475569", color: "#ffffff" },
  odp: { label: "ODP", background: "#475569", color: "#ffffff" },
};

const FALLBACK: FormatBadge = { label: "BOOK", background: "#6b7280", color: "#ffffff" };

/** 归一化格式值（小写、去点），返回书签配色。 */
export function formatBadge(format: string | null | undefined): FormatBadge {
  const key = (format ?? "").trim().toLowerCase().replace(/^\./, "");
  return PALETTE[key] ?? { ...FALLBACK, label: key ? key.toUpperCase().slice(0, 4) : FALLBACK.label };
}
