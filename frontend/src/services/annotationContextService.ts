// F-8-001 上下文标注 service 封装。
// 对应后端 src-tauri/src/commands/annotation.rs 的 save_annotation_context。
// 用途：为 AI 教练 / 引用卡片回填「引用起止页码 + 上下文摘录」，全字段可选（None 不动）。

import { CMD, invoke } from "./tauri";

export interface AnnotationContextPayload {
  annotationId: string;
  contextStartPage?: number | null;
  contextEndPage?: number | null;
  contextExcerpt?: string | null;
}

export const annotationContextService = {
  /** 保存上下文标注（字段 None 保持原值，不修改） */
  async save(payload: AnnotationContextPayload): Promise<void> {
    return invoke<void>(CMD.saveAnnotationContext, {
      annotationId: payload.annotationId,
      contextStartPage: payload.contextStartPage ?? null,
      contextEndPage: payload.contextEndPage ?? null,
      contextExcerpt: payload.contextExcerpt ?? null,
    });
  },
};