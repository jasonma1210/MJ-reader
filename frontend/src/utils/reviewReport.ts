/** 复盘报告结构化 JSON 解析（后端 ReviewReport.report 为 snake_case JSON → 归一化为 camelCase 前端对象） */
export interface ReviewReportJson {
  reviewTitle?: string;
  reviewType?: string;
  masteredKnowledge?: string[];
  weakKnowledge?: Array<{
    nodeId?: string;
    knowledgeSummary?: string;
    errorSummary?: string;
    chapterIndex?: number;
    chapterTitle?: string;
  }>;
  memoryCards?: Array<{ cardFront?: string; cardBack?: string; nodeId?: string; chapter?: string }>;
  selfTestQuestions?: Array<{
    question?: string;
    options?: string[];
    answer?: string;
    explanation?: string;
    chapter?: string;
  }>;
  suggestion?: string[];
  [key: string]: unknown;
}

function toCamel(s: string): string {
  return s.replace(/_([a-z])/g, (_, c: string) => c.toUpperCase());
}

/** 递归把对象键 snake_case → camelCase（数组元素同样处理） */
function normalize(obj: unknown): unknown {
  if (Array.isArray(obj)) return obj.map(normalize);
  if (obj && typeof obj === "object") {
    const out: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(obj as Record<string, unknown>)) {
      out[toCamel(k)] = normalize(v);
    }
    return out;
  }
  return obj;
}

export function parseReviewReport(raw: string | null | undefined): ReviewReportJson | null {
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object") return null;
    return normalize(parsed) as ReviewReportJson;
  } catch {
    return null;
  }
}
