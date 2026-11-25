import { useTranslation } from "react-i18next";
import {
  AtSign,
  CornerUpLeft,
  CornerUpRight,
  ExternalLink,
  FileText,
  Link2,
  Pencil,
  Sparkles,
  X,
} from "lucide-react";
import {
  ACTION_ICONS,
  WB_ACTION_LABELS,
  WB_SOURCE_LABELS,
  type WhiteboardActionId,
} from "./WhiteboardCardNode";
import type {
  WhiteboardCardNode as NodeData,
  WhiteboardLink,
} from "../../services/whiteboardService";
import { isHttpUrl } from "../../utils/cardSecurity";
import { cn } from "../../utils/cn";

/**
 * 统一卡片检查器（P1）：内容 / 关系 / AI 三 Tab。
 * 手机竖屏用底部抽屉（60% 高度上限），平板横屏 / PC 用右侧面板。
 * 从节点气泡收进来的能力：来源分类、上一程/下一程、外链、掌握度、单卡 AI、反向关联。
 * 节点上只保留高频的编辑 / 折叠 / 删除。
 */
export type InspectorTab = "content" | "relation" | "ai";

const INSPECTOR_SOURCES = [
  "note",
  "highlight",
  "knowledge",
  "conceptCard",
  "misquestion",
] as const;

interface WhiteboardInspectorProps {
  node: NodeData;
  /** 端侧容器：手机竖屏底部抽屉，其它端右侧面板 */
  variant: "bottom" | "right";
  tab: InspectorTab;
  onTabChange: (tab: InspectorTab) => void;
  onClose: () => void;
  onChangeSource: (nodeId: string, source: string) => void;
  onEdit: (node: NodeData) => void;
  onLinkRequest: (node: NodeData, dir: "parent" | "child") => void;
  onAction: (node: NodeData, actionId: WhiteboardActionId) => void;
  actionBusy: boolean;
  /** 画布全量节点：用于计算反向关联与出链 */
  nodes: NodeData[];
  links: WhiteboardLink[];
  onSelectRelated: (nodeId: string) => void;
  onMentionRef: (cardId: string) => void;
}

const TAB_DEFS: Array<{ id: InspectorTab; labelKey: string; icon: React.ReactNode }> = [
  { id: "content", labelKey: "whiteboard.inspector.tab.content", icon: <FileText className="h-4 w-4" /> },
  { id: "relation", labelKey: "whiteboard.inspector.tab.relation", icon: <Link2 className="h-4 w-4" /> },
  { id: "ai", labelKey: "whiteboard.inspector.tab.ai", icon: <Sparkles className="h-4 w-4" /> },
];

export function WhiteboardInspector({
  node,
  variant,
  tab,
  onTabChange,
  onClose,
  onChangeSource,
  onEdit,
  onLinkRequest,
  onAction,
  actionBusy,
  nodes,
  links,
  onSelectRelated,
  onMentionRef,
}: WhiteboardInspectorProps) {
  const { t } = useTranslation();
  const mastery = node.card?.masteryScore;
  const title = node.card?.title || node.card?.body || node.cardId;
  const externalUrl =
    node.card?.noteType === "web" || node.card?.noteType === "onlineVideo"
      ? node.card.body || ""
      : "";

  const incoming = links
    .filter((l) => l.to === node.id)
    .map((l) => nodes.find((n) => n.id === l.from))
    .filter((n): n is NodeData => !!n);
  const outgoing = links
    .filter((l) => l.from === node.id)
    .map((l) => nodes.find((n) => n.id === l.to))
    .filter((n): n is NodeData => !!n);
  const mentionRef = `(#${node.cardId})`;
  const mentions = nodes.filter(
    (n) => n.id !== node.id && !!n.card?.body && n.card.body.includes(mentionRef),
  );

  return (
    <div
      role="dialog"
      aria-label={t("whiteboard.inspector.title")}
      className={cn(
        "flex flex-col border-line bg-paper shadow-xl",
        variant === "bottom"
          ? "wb-inspector-drawer absolute inset-x-0 bottom-0 z-[65] max-h-[60%] rounded-t-[var(--radius-md)] border-t"
          : "absolute inset-y-0 right-0 z-[65] w-80 max-w-[85vw] border-l",
      )}
      onPointerDown={(e) => e.stopPropagation()}
      onDoubleClick={(e) => e.stopPropagation()}
      onContextMenu={(e) => {
        e.stopPropagation();
        e.preventDefault();
      }}
    >
      {/* 标题 + Tab + 关闭 */}
      <div className="shrink-0 border-b border-line px-3 pt-2">
        <div className="flex items-center gap-2">
          <p className="min-w-0 flex-1 truncate text-sm font-semibold text-ink" title={title}>
            {title}
          </p>
          <button
            type="button"
            onClick={onClose}
            aria-label={t("common.close")}
            className="rounded-[var(--radius-md)] p-1.5 text-ink-muted transition hover:bg-paper-soft"
          >
            <X className="h-4 w-4" />
          </button>
        </div>
        <div className="mt-2 flex gap-1" role="tablist">
          {TAB_DEFS.map((def) => (
            <button
              key={def.id}
              type="button"
              role="tab"
              aria-selected={tab === def.id}
              onClick={() => onTabChange(def.id)}
              className={cn(
                "flex min-w-0 flex-1 items-center justify-center gap-1 rounded-t-[var(--radius-md)] px-2 py-2 text-xs transition",
                tab === def.id
                  ? "border-b-2 border-accent bg-paper-soft font-medium text-accent"
                  : "border-b-2 border-transparent text-ink-muted hover:bg-paper-soft",
              )}
            >
              {def.icon}
              <span className="truncate">{t(def.labelKey)}</span>
            </button>
          ))}
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-auto p-3">
        {tab === "content" && (
          <div className="space-y-3">
            <div>
              <p className="mb-1 text-[11px] font-medium text-ink-muted">
                {t("whiteboard.inspector.source")}
              </p>
              <select
                value={node.source}
                onChange={(e) => onChangeSource(node.id, e.target.value)}
                aria-label={t("whiteboard.inspector.source")}
                className="w-full rounded-[var(--radius-md)] border border-line bg-paper-soft px-2 py-2 text-xs text-ink outline-none focus:border-accent"
              >
                {INSPECTOR_SOURCES.map((s) => (
                  <option key={s} value={s}>
                    {t(WB_SOURCE_LABELS[s])}
                  </option>
                ))}
              </select>
            </div>

            <div className="flex items-center justify-between rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2">
              <span className="text-xs text-ink-muted">{t("mastery.masteryScore")}</span>
              <span className="text-sm font-medium text-ink">
                {mastery === undefined || mastery === null
                  ? "—"
                  : `${Math.round(mastery * 100)}%`}
              </span>
            </div>

            <button
              type="button"
              onClick={() => onEdit(node)}
              className="flex w-full items-center gap-2 rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2.5 text-left text-xs text-ink transition active:bg-line"
            >
              <Pencil className="h-4 w-4 shrink-0 text-ink-muted" />
              {t("whiteboard.editCard")}
            </button>

            {externalUrl && isHttpUrl(externalUrl) && (
              <a
                href={externalUrl}
                target="_blank"
                rel="noreferrer"
                className="flex w-full items-center gap-2 rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2.5 text-xs text-ink transition active:bg-line"
              >
                <ExternalLink className="h-4 w-4 shrink-0 text-ink-muted" />
                {t("whiteboard.pinBlocked.openInNew")}
              </a>
            )}

            {node.card?.body && (
              <p className="rounded-[var(--radius-md)] bg-paper-soft px-3 py-2 text-xs leading-relaxed text-ink-muted">
                {node.card.body}
              </p>
            )}
          </div>
        )}

        {tab === "relation" && (
          <div className="space-y-3">
            <div className="grid grid-cols-2 gap-2">
              <button
                type="button"
                onClick={() => onLinkRequest(node, "parent")}
                className="flex items-center justify-center gap-1 rounded-[var(--radius-md)] border border-line bg-paper-soft px-2 py-2.5 text-xs text-ink transition active:bg-line"
              >
                <CornerUpLeft className="h-4 w-4 shrink-0 text-warning" />
                {t("whiteboard.linkParent")}
              </button>
              <button
                type="button"
                onClick={() => onLinkRequest(node, "child")}
                className="flex items-center justify-center gap-1 rounded-[var(--radius-md)] border border-line bg-paper-soft px-2 py-2.5 text-xs text-ink transition active:bg-line"
              >
                <CornerUpRight className="h-4 w-4 shrink-0 text-success" />
                {t("whiteboard.linkChild")}
              </button>
            </div>

            <InspectorList
              title={t("whiteboard.backlinkIncoming")}
              icon={<Link2 className="h-3.5 w-3.5 shrink-0 text-ink-muted" />}
              items={incoming}
              onSelectRelated={onSelectRelated}
            />
            <InspectorList
              title={t("whiteboard.inspector.outgoing")}
              icon={<CornerUpRight className="h-3.5 w-3.5 shrink-0 text-success" />}
              items={outgoing}
              onSelectRelated={onSelectRelated}
            />
            <InspectorList
              title={t("whiteboard.backlinkMentions")}
              icon={<AtSign className="h-3.5 w-3.5 shrink-0 text-accent" />}
              items={mentions}
              onSelectRelated={onSelectRelated}
              onMentionRef={onMentionRef}
            />
          </div>
        )}

        {tab === "ai" && (
          <div className="space-y-2">
            <p className="text-[11px] leading-relaxed text-ink-muted">
              {t("whiteboard.inspector.aiHint")}
            </p>
            {(Object.keys(WB_ACTION_LABELS) as WhiteboardActionId[]).map((id) => (
              <button
                key={id}
                type="button"
                disabled={actionBusy}
                onClick={() => onAction(node, id)}
                className="flex w-full items-center gap-2 rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2.5 text-left text-xs text-ink transition active:bg-line disabled:opacity-50"
              >
                <span className="shrink-0 text-accent">{ACTION_ICONS[id]}</span>
                {t(WB_ACTION_LABELS[id])}
              </button>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

/** 关系 Tab 的可点击卡片列表：空态给出明确提示，避免用户误以为关系丢失 */
function InspectorList({
  title,
  icon,
  items,
  onSelectRelated,
  onMentionRef,
}: {
  title: string;
  icon: React.ReactNode;
  items: NodeData[];
  onSelectRelated: (nodeId: string) => void;
  onMentionRef?: (cardId: string) => void;
}) {
  const { t } = useTranslation();
  return (
    <div>
      <p className="mb-1 px-0.5 text-[11px] font-semibold uppercase tracking-wide text-ink-muted">
        {title}
      </p>
      {items.length === 0 ? (
        <p className="rounded-[var(--radius-md)] bg-paper-soft px-3 py-2 text-[11px] text-ink-muted">
          {t("whiteboard.inspector.relationEmpty")}
        </p>
      ) : (
        <div className="space-y-1">
          {items.map((n) => (
            <button
              key={n.id}
              type="button"
              onClick={() => {
                onSelectRelated(n.id);
                onMentionRef?.(n.cardId);
              }}
              className="flex w-full items-center gap-2 rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2 text-left text-xs text-ink transition active:bg-line"
            >
              {icon}
              <span className="min-w-0 flex-1 truncate">
                {n.card?.title || n.card?.body || n.cardId}
              </span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
