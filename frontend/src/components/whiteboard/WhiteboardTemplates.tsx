import { useTranslation } from "react-i18next";
import { BookOpen, Map, Network, NotebookPen, X } from "lucide-react";
import { cn } from "../../utils/cn";

/**
 * 白板模板（P2）：读书笔记 / 概念图 / 错题本 / 章节地图。
 * 只影响「新建卡片」的标题前缀与初始正文结构（布局建议由文案提示），
 * 不自动铺卡、不写库、不改动任何已有卡片。
 */
export type BoardTemplateId = "reading" | "concept" | "mistake" | "chapter";

interface TemplateDef {
  id: BoardTemplateId;
  nameKey: string;
  descKey: string;
  icon: React.ReactNode;
}

const TEMPLATES: TemplateDef[] = [
  {
    id: "reading",
    nameKey: "whiteboard.template.reading.name",
    descKey: "whiteboard.template.reading.desc",
    icon: <BookOpen className="h-5 w-5" />,
  },
  {
    id: "concept",
    nameKey: "whiteboard.template.concept.name",
    descKey: "whiteboard.template.concept.desc",
    icon: <Network className="h-5 w-5" />,
  },
  {
    id: "mistake",
    nameKey: "whiteboard.template.mistake.name",
    descKey: "whiteboard.template.mistake.desc",
    icon: <NotebookPen className="h-5 w-5" />,
  },
  {
    id: "chapter",
    nameKey: "whiteboard.template.chapter.name",
    descKey: "whiteboard.template.chapter.desc",
    icon: <Map className="h-5 w-5" />,
  },
];

interface WhiteboardTemplatesProps {
  open: boolean;
  onClose: () => void;
  active: BoardTemplateId | null;
  onPick: (id: BoardTemplateId | null) => void;
}

export function WhiteboardTemplates({
  open,
  onClose,
  active,
  onPick,
}: WhiteboardTemplatesProps) {
  const { t } = useTranslation();
  if (!open) return null;
  return (
    <div
      className="fixed inset-0 z-[90] flex items-center justify-center bg-black/40 p-6"
      onClick={onClose}
      onPointerDown={(e) => e.stopPropagation()}
      role="presentation"
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label={t("whiteboard.template.title")}
        className="w-full max-w-md rounded-[var(--radius-md)] border border-line bg-paper p-4 shadow-xl"
        onClick={(e) => e.stopPropagation()}
        onDoubleClick={(e) => e.stopPropagation()}
        onContextMenu={(e) => e.preventDefault()}
      >
        <div className="mb-3 flex items-start justify-between gap-2">
          <div className="min-w-0">
            <h2 className="text-sm font-semibold text-ink">
              {t("whiteboard.template.title")}
            </h2>
            <p className="mt-1 text-xs leading-relaxed text-ink-muted">
              {t("whiteboard.template.hint")}
            </p>
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label={t("common.close")}
            className="shrink-0 rounded-[var(--radius-md)] p-1 text-ink-muted transition hover:bg-paper-soft"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        <div className="grid grid-cols-2 gap-2">
          {TEMPLATES.map((def) => (
            <button
              key={def.id}
              type="button"
              onClick={() => {
                onPick(def.id);
                onClose();
              }}
              className={cn(
                "flex flex-col items-start gap-1.5 rounded-[var(--radius-md)] border px-3 py-3 text-left transition active:bg-paper-soft",
                active === def.id
                  ? "border-accent bg-accent/10"
                  : "border-line bg-paper-soft",
              )}
            >
              <span className={active === def.id ? "text-accent" : "text-ink-muted"}>
                {def.icon}
              </span>
              <span className="text-xs font-medium text-ink">{t(def.nameKey)}</span>
              <span className="text-[11px] leading-relaxed text-ink-muted">
                {t(def.descKey)}
              </span>
            </button>
          ))}
        </div>

        <button
          type="button"
          onClick={() => {
            onPick(null);
            onClose();
          }}
          className="mt-3 w-full rounded-[var(--radius-md)] border border-line px-3 py-2 text-xs text-ink-muted transition active:bg-paper-soft"
        >
          {t("whiteboard.template.none")}
        </button>
      </div>
    </div>
  );
}
