import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown } from "lucide-react";
import { graphService, type GraphNode } from "../../services/graphService";
import { LoadingState } from "../common/states";
import { ErrorState } from "../common/states";
import { cn } from "../../utils/cn";

/**
 * 共用知识节点选择器（场景化练习 / 语音问答 / 教学相长 三页复用）。
 * 数据源：graphService.get()（知识图谱聚合全部知识点，仅 Tauri 内可读，
 * 浏览器预览为空 → 渲染空态提示）。选择结果 value 为节点 id，"" 表示不指定。
 */
interface KnowledgeNodeSelectProps {
  value: string;
  onChange: (nodeId: string) => void;
  /** 是否允许"不指定"选项（默认 true） */
  allowEmpty?: boolean;
  label?: string;
  className?: string;
}

export function KnowledgeNodeSelect({
  value,
  onChange,
  allowEmpty = true,
  label,
  className,
}: KnowledgeNodeSelectProps) {
  const { t } = useTranslation();
  const [nodes, setNodes] = useState<GraphNode[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    void graphService
      .get()
      .then((g) => {
        if (alive) setNodes(g?.nodes ?? []);
      })
      .catch((e) => {
        if (alive) {
          setError(e && typeof e === "object" && "message" in e ? String((e as { message: unknown }).message) : String(e));
        }
      });
    return () => {
      alive = false;
    };
  }, []);

  return (
    <div className={cn("flex flex-col gap-1.5", className)}>
      {(label || allowEmpty) && (
        <label className="text-xs font-medium text-ink-soft">
          {label ?? t("practice.targetNode")}
        </label>
      )}
      <div className="relative">
        {nodes === null && !error ? (
          <div className="flex h-10 items-center rounded-[var(--radius-md)] border border-line bg-paper-soft px-3">
            <LoadingState label={t("practice.loadingNodes")} fill={false} className="gap-1 py-0" />
          </div>
        ) : error ? (
          <ErrorState message={error} className="p-2" />
        ) : (nodes?.length ?? 0) === 0 ? (
          <div className="h-10 rounded-[var(--radius-md)] border border-line bg-paper-soft px-3">
            <span className="flex h-full items-center text-xs text-ink-muted">
              {t("practice.noNodes")}
            </span>
          </div>
        ) : (
          <>
            <select
              value={value}
              onChange={(e) => onChange(e.target.value)}
              aria-label={label ?? t("practice.targetNode")}
              className="h-10 w-full appearance-none rounded-[var(--radius-md)] border border-line bg-paper px-3 pr-9 text-sm text-ink outline-none transition focus-visible:ring-2 focus-visible:ring-accent/40"
            >
              {allowEmpty && (
                <option value="">{t("practice.noNodePick")}</option>
              )}
              {(nodes ?? []).map((n) => (
                <option key={n.id} value={n.id}>
                  {n.label}
                </option>
              ))}
            </select>
            <ChevronDown className="pointer-events-none absolute right-3 top-1/2 h-4 w-4 -translate-y-1/2 text-ink-muted" />
          </>
        )}
      </div>
    </div>
  );
}