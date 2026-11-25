/**
 * 成卡确认页纯逻辑：草稿状态机、来源锚点构建、路由参数解析。
 *
 * 设计约束（读后记忆系统 §3.2 A / §10）：
 * - AI 草稿默认进「待核验」，绝不静默入库；
 * - 每张卡必须有来源锚点（bookId + cfi/page/highlight 至少一项）才能采纳；
 * - 作废必须带枚举原因码（offSource/wrongAnswer/unimportant），不传原文。
 *
 * 这里只放可单测的纯函数，不触碰 React / Tauri，便于 vitest（node 环境）直接验证。
 */

export type RejectReason = "offSource" | "wrongAnswer" | "unimportant";
export type DraftStatus = "pending" | "accepted" | "rejected";
export type ConfirmSource = "highlight" | "report" | "manual";

export const REJECT_REASONS: readonly RejectReason[] = [
  "offSource",
  "wrongAnswer",
  "unimportant",
];

/** 与 cardService.generateFlashcardDrafts 草稿结构对齐的本地视图模型。 */
export interface DraftCard {
  id: string;
  question: string;
  answer: string;
  chapter: string | null;
  cfi: string | null;
  pageIndex: number | null;
  highlightId: string | null;
  sourceType: string | null;
  bookId: string | null;
  confidence: number | null;
  selectedText: string | null;
}

/** 单张草稿在确认页的完整状态。 */
export interface DraftViewState {
  draft: DraftCard;
  status: DraftStatus;
  /** 用户改过问题/答案 → 埋点 ai_card_edited，不再重复报 proposed。 */
  edited: boolean;
  question: string;
  answer: string;
  rejectReason: RejectReason | null;
  /** 采纳后挂接的概念节点 id（可选）。 */
  nodeId: string | null;
  /** 采纳成功后落库的卡片 id。 */
  cardId: string | null;
  /** 批量采纳勾选态。 */
  checked: boolean;
}

export type DraftAction =
  | { type: "init"; drafts: DraftCard[] }
  | { type: "edit"; id: string; question: string; answer: string }
  | { type: "reject"; id: string; reason: RejectReason }
  | { type: "accept"; id: string; cardId: string }
  | { type: "toggleCheck"; id: string }
  | { type: "setNode"; id: string; nodeId: string | null };

function draftToView(draft: DraftCard): DraftViewState {
  return {
    draft,
    status: "pending",
    edited: false,
    question: draft.question,
    answer: draft.answer,
    rejectReason: null,
    nodeId: null,
    cardId: null,
    checked: false,
  };
}

export function draftReducer(
  state: DraftViewState[],
  action: DraftAction,
): DraftViewState[] {
  switch (action.type) {
    case "init":
      return action.drafts.map(draftToView);
    case "edit":
      return state.map((s) =>
        s.draft.id === action.id && s.status === "pending"
          ? {
              ...s,
              question: action.question,
              answer: action.answer,
              edited:
                s.edited ||
                action.question !== s.draft.question ||
                action.answer !== s.draft.answer,
            }
          : s,
      );
    case "reject":
      return state.map((s) =>
        s.draft.id === action.id && s.status === "pending"
          ? { ...s, status: "rejected", rejectReason: action.reason, checked: false }
          : s,
      );
    case "accept":
      return state.map((s) =>
        s.draft.id === action.id && s.status === "pending"
          ? { ...s, status: "accepted", cardId: action.cardId, checked: false }
          : s,
      );
    case "toggleCheck":
      return state.map((s) =>
        s.draft.id === action.id && s.status === "pending"
          ? { ...s, checked: !s.checked }
          : s,
      );
    case "setNode":
      return state.map((s) =>
        s.draft.id === action.id && s.status === "pending"
          ? { ...s, nodeId: action.nodeId }
          : s,
      );
    default:
      return state;
  }
}

/**
 * 构建 sourceLocator JSON（六字段齐全，缺值写 null）。
 * valid=false 时禁止入库：bookId 必填，且 cfiRange/pageIndex/highlightId 至少一项有值。
 */
export function buildSourceLocator(
  draft: DraftCard,
  fallbackBookId: string,
): { json: string; valid: boolean } {
  const cfiRange = draft.cfi?.trim() || null;
  const highlightId = draft.highlightId?.trim() || null;
  const bookId = draft.bookId?.trim() || fallbackBookId.trim() || null;
  const pageIndex =
    typeof draft.pageIndex === "number" && Number.isFinite(draft.pageIndex)
      ? draft.pageIndex
      : null;
  const locator = {
    cfiRange,
    pageIndex,
    highlightId,
    sourceType: draft.sourceType?.trim() || "highlight",
    bookId,
    chapter: draft.chapter?.trim() || null,
  };
  const valid = Boolean(bookId && (cfiRange || highlightId || pageIndex !== null));
  return { json: JSON.stringify(locator), valid };
}

/** 路由参数：从 query string 解析成卡确认入口参数。 */
export interface CardConfirmParams {
  bookId: string;
  highlightIds: string[];
  source: ConfirmSource;
}

export function parseCardConfirmParams(search: string): CardConfirmParams {
  const p = new URLSearchParams(search);
  const rawSource = p.get("source");
  const source: ConfirmSource =
    rawSource === "highlight" || rawSource === "report" || rawSource === "manual"
      ? rawSource
      : "highlight";
  return {
    bookId: (p.get("bookId") ?? "").trim(),
    highlightIds: (p.get("highlightIds") ?? "")
      .split(",")
      .map((x) => x.trim())
      .filter(Boolean),
    source,
  };
}

/** 入口统一路径构造器，保证高亮列表/复盘面板生成同一种 URL。 */
export function buildCardConfirmPath(params: CardConfirmParams): string {
  const p = new URLSearchParams();
  p.set("bookId", params.bookId);
  if (params.highlightIds.length > 0) {
    p.set("highlightIds", params.highlightIds.join(","));
  }
  p.set("source", params.source);
  return `/cards/new?${p.toString()}`;
}

/**
 * 服务草稿 → 本地视图模型。
 * 兼容 cfi / cfiRange 两种字段名，缺失一律归一为 null，避免把 undefined 写进 JSON。
 * 不用 any：显式列出允许字段，结构未知时走默认值。
 */
export interface RawDraftLike {
  id?: string | null;
  question?: string | null;
  /** 后端 generate_flashcard_drafts 的问题字段名。 */
  front?: string | null;
  answer?: string | null;
  /** 后端 generate_flashcard_drafts 的答案字段名。 */
  back?: string | null;
  chapter?: string | null;
  cfi?: string | null;
  cfiRange?: string | null;
  pageIndex?: number | null;
  highlightId?: string | null;
  sourceType?: string | null;
  bookId?: string | null;
  confidence?: number | null;
  selectedText?: string | null;
  /** 后端草稿附带成本信息，视图模型不使用但要保持结构兼容。 */
  estimatedTokens?: number | null;
  provider?: string | null;
}

function text(value: string | null | undefined): string {
  return value?.trim() ?? "";
}

export function toDraftCard(
  raw: RawDraftLike,
  index: number,
  fallbackBookId: string,
): DraftCard {
  return {
    id: text(raw.id) || `draft-${index}`,
    question: text(raw.question ?? raw.front),
    answer: text(raw.answer ?? raw.back),
    chapter: text(raw.chapter) || null,
    cfi: text(raw.cfi ?? raw.cfiRange) || null,
    pageIndex:
      typeof raw.pageIndex === "number" && Number.isFinite(raw.pageIndex)
        ? raw.pageIndex
        : null,
    highlightId: text(raw.highlightId) || null,
    sourceType: text(raw.sourceType) || "highlight",
    bookId: text(raw.bookId) || fallbackBookId || null,
    confidence:
      typeof raw.confidence === "number" && Number.isFinite(raw.confidence)
        ? raw.confidence
        : null,
    selectedText: text(raw.selectedText) || null,
  };
}

/** 已编辑的卡才用编辑后文本落库；未编辑保持草稿原文。 */
export function cardContentOf(view: DraftViewState): { title: string; content: string } {
  return {
    title: view.question.trim() || view.draft.question,
    content: view.answer.trim() || view.draft.answer,
  };
}
