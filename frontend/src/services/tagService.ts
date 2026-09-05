// F-7-003 标签体系 service 封装。
// 对应后端 src-tauri/src/commands/tags.rs 的 serde 结构（rename_all=camelCase）。
// 仅通过 CMD.* 引用命令名，统一走 invoke。

import { CMD, invoke, isTauri } from "./tauri";

/** 标签树节点（children 为嵌套子节点） */
export interface TagNode {
  id: string;
  name: string;
  parentId: string | null;
  color: string;
  icon: string;
  sortOrder: number;
  children: TagNode[];
}

/** 标签打标作用域 */
export type TagScope =
  | "book"
  | "highlight"
  | "note"
  | "knowledge"
  | "card"
  | "misquestion"
  | "whiteCard";

/** 标签预设配色（对齐后端 default_color 帕尔塔） */
export const TAG_COLOR_PALETTE = [
  "#8a94a6",
  "#7f9a8d",
  "#8a7f9a",
  "#9a8f7a",
  "#7f9a9a",
  "#9a7f8f",
];

export const tagService = {
  /** 标签树（空库时后端会预置 6 个默认根标签） */
  async getTree(): Promise<TagNode[]> {
    if (!isTauri()) return [];
    return invoke<TagNode[]>(CMD.tagsGetTree);
  },

  /** 新建标签（parentId/color 可选；后端仅校验 name） */
  async create(
    name: string,
    parentId?: string | null,
    color?: string,
  ): Promise<TagNode> {
    return invoke<TagNode>(CMD.tagsCreate, {
      name,
      parentId: parentId || null,
      color: color || null,
    });
  },

  /** 重命名标签（含同级重名校验） */
  async rename(tagId: string, name: string): Promise<void> {
    return invoke<void>(CMD.tagsRename, { tagId, name });
  },

  /** 删除标签；mergeToId 可选（存在时先迁移 content_tags 再删） */
  async delete(tagId: string, mergeToId?: string | null): Promise<void> {
    return invoke<void>(CMD.tagsDelete, {
      tagId,
      mergeToId: mergeToId || null,
    });
  },

  /** AI 建议打标：返回标签名列表（不落库，落库交给 apply） */
  async suggest(
    scope: string,
    scopeId: string,
    text?: string,
  ): Promise<string[]> {
    if (!isTauri()) return [];
    return invoke<string[]>(CMD.tagsSuggest, { scope, scopeId, text: text ?? null });
  },

  /** 打标落库：把标签名映射到 tags 并写入 content_tags */
  async apply(
    scope: string,
    scopeId: string,
    tagNames: string[],
    isAuto?: boolean,
  ): Promise<string[]> {
    return invoke<string[]>(CMD.tagsApply, {
      scope,
      scopeId,
      tagNames,
      isAuto: isAuto ?? false,
    });
  },

  /** 查询某实体已打的标签 */
  async listFor(scope: string, scopeId: string): Promise<TagNode[]> {
    if (!isTauri()) return [];
    return invoke<TagNode[]>(CMD.tagsListFor, { scope, scopeId });
  },

  /** 删除某实体上某标签的关联 */
  async remove(scope: string, scopeId: string, tagId: string): Promise<void> {
    return invoke<void>(CMD.tagsRemove, { scope, scopeId, tagId });
  },

  /** 按名称模糊搜索标签 */
  async search(keyword: string): Promise<TagNode[]> {
    if (!isTauri()) return [];
    return invoke<TagNode[]>(CMD.tagsSearch, { keyword });
  },
};