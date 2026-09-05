import { useCallback, useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { askConfirm } from "../components/ui/confirmService";
import { useTranslation } from "react-i18next";
import { SubBackHeader } from "../components/shell/SubBackHeader";
import {
  Plus,
  Play,
  ChevronRight,
  Check,
  Circle,
  Trash2,
  Pencil,
  History,
  Loader2,
  ArrowLeft,
  Layers,
  X,
  Crosshair,
} from "lucide-react";
import { Button } from "../components/ui/Button";
import { Surface } from "../components/ui/Surface";
import { Sheet } from "../components/ui/Sheet";
import { EmptyState, LoadingState, ErrorState } from "../components/common/states/index";
import { errMsg, toast } from "../utils/toast";
import { cn } from "../utils/cn";
import {
  learningPathService,
  PATH_NODE_STATUS,
  type LearningPath,
  type PathNode,
  type PathNodeUpdate,
  type PathAdjustment,
} from "../services/learningPathService";
import { useLibraryStore } from "../stores/libraryStore";

const STATUS_STYLE: Record<string, string> = {
  pending: "border-line text-ink-muted",
  in_progress: "border-accent text-accent ring-1 ring-accent/40",
  completed: "border-accent bg-accent text-accent-fg",
  skipped: "border-line text-ink-muted line-through",
  supplemented: "border-accent bg-accent-bg text-accent",
};

export function LearningPathPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [paths, setPaths] = useState<LearningPath[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [genOpen, setGenOpen] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setPaths(await learningPathService.list());
    } catch (e) {
      setError(errMsg(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const select = async (id: string) => {
    setSelectedId(id);
  };

  const removePath = async (p: LearningPath) => {
    if (!(await askConfirm(t("path.deleteConfirm", { title: p.title })))) return;
    try {
      await learningPathService.remove(p.id);
      if (selectedId === p.id) setSelectedId(null);
      await load();
    } catch (e) {
      toast(errMsg(e));
    }
  };

  if (selectedId) {
    return (
      <PathDetail
        pathId={selectedId}
        onBack={() => {
          setSelectedId(null);
          void load();
        }}
        onDeleted={() => {
          setSelectedId(null);
          void load();
        }}
      />
    );
  }

  return (
    <div className="flex h-full flex-col gap-4 overflow-auto bg-paper px-4 pb-4 pt-3">
      {/* 二级页返回栏（2026-09-04 用户反馈：学习路径无返回键） */}
      <SubBackHeader titleKey="path.title" onBack={() => navigate(-1)} />
      <div className="flex items-center justify-between">
        <h1 className="font-extrabold text-ink" style={{ fontSize: "var(--fs-appbar-h1)" }}>
          {t("path.title")}
        </h1>
        <Button size="sm" iconLeft={<Plus className="h-4 w-4" />} onClick={() => setGenOpen(true)}>
          {t("path.new")}
        </Button>
      </div>

      {loading ? (
        <LoadingState />
      ) : error ? (
        <ErrorState message={error} onRetry={() => void load()} />
      ) : paths.length === 0 ? (
        <EmptyState
          title={t("path.empty")}
          description={t("path.emptyDesc")}
          icon={Layers}
          action={
            <Button iconLeft={<Plus className="h-4 w-4" />} onClick={() => setGenOpen(true)}>
              {t("path.new")}
            </Button>
          }
        />
      ) : (
        <div className="flex flex-col gap-3">
          {paths.map((p) => {
            const done = p.nodes.filter((n) => n.status === "completed").length;
            return (
              <Surface key={p.id} pad="md" className="transition active:scale-[0.99]">
                <button
                  className="flex w-full items-center gap-3 text-left"
                  onClick={() => void select(p.id)}
                >
                  <div
                    className={cn(
                      "grid h-11 w-11 shrink-0 place-items-center rounded-[var(--radius-md)]",
                      p.isActive ? "bg-accent text-accent-fg" : "bg-paper-soft text-ink-soft",
                    )}
                  >
                    <Layers className="h-5 w-5" />
                  </div>
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <span className="truncate text-sm font-bold text-ink">{p.title}</span>
                      {p.isActive && (
                        <span className="shrink-0 rounded-full bg-accent px-2 py-0.5 text-[10px] font-semibold text-accent-fg">
                          {t("path.active")}
                        </span>
                      )}
                    </div>
                    <div className="mt-0.5 truncate text-xs text-ink-muted">{p.goal}</div>
                    <div className="mt-1 flex items-center gap-1 text-[11px] text-ink-muted">
                      <span>
                        {done}/{p.nodes.length} {t("path.nodesDone")}
                      </span>
                    </div>
                  </div>
                  <ChevronRight className="h-5 w-5 shrink-0 text-ink-muted" />
                </button>

                <div className="mt-3 flex items-center gap-2 border-t border-line pt-3">
                  <Button size="sm" variant="secondary" onClick={() => void select(p.id)}>
                    {t("path.open")}
                  </Button>
                  {!p.isActive && (
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={() =>
                        void learningPathService
                          .activate(p.id)
                          .then(() => load())
                          .then(() => toast(t("path.activated")))
                          .catch((e) => toast(errMsg(e)))
                      }
                    >
                      {t("path.activateAction")}
                    </Button>
                  )}
                  <span className="flex-1" />
                  <Button
                    size="sm"
                    variant="ghost"
                    iconLeft={<Trash2 className="h-4 w-4" />}
                    onClick={() => void removePath(p)}
                  >
                    {t("common.delete")}
                  </Button>
                </div>
              </Surface>
            );
          })}
        </div>
      )}

      <GenSheet open={genOpen} onClose={() => setGenOpen(false)} onCreated={(id) => select(id)} />
    </div>
  );
}

/** 新建路径表单（目标 + 素材来源多选） */
function GenSheet({
  open,
  onClose,
  onCreated,
}: {
  open: boolean;
  onClose: () => void;
  onCreated: (id: string) => void;
}) {
  const { t } = useTranslation();
  const books = useLibraryStore((s) => s.books);
  const [goal, setGoal] = useState("");
  const [materials, setMaterials] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (open && books.length === 0) void useLibraryStore.getState().load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  const toggle = (id: string) =>
    setMaterials((v) => (v.includes(id) ? v.filter((x) => x !== id) : [...v, id]));

  const submit = async () => {
    if (!goal.trim()) return;
    setBusy(true);
    try {
      const p = await learningPathService.generate(materials, goal.trim());
      setGoal("");
      setMaterials([]);
      onClose();
      toast(t("path.generated"));
      onCreated(p.id);
    } catch (e) {
      toast(errMsg(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Sheet open={open} onClose={onClose} title={t("path.new")}>
      <div className="flex flex-col gap-4">
        <label className="flex flex-col gap-1 text-xs text-ink-muted">
          {t("path.goal")}
          <textarea
            value={goal}
            onChange={(e) => setGoal(e.target.value)}
            rows={3}
            placeholder={t("path.goalPlaceholder")}
            className="h-auto resize-none rounded-[var(--radius-md)] border border-line bg-paper p-3 text-sm text-ink outline-none focus:border-accent"
          />
        </label>
        <div className="flex flex-col gap-1 text-xs text-ink-muted">
          {t("path.materials")}
          {books.length === 0 ? (
            <p className="text-ink-muted">{t("path.noMaterials")}</p>
          ) : (
            <div className="flex flex-col gap-1.5">
              {books.map((b) => (
                <button
                  key={b.id}
                  onClick={() => toggle(b.id)}
                  className={cn(
                    "flex items-center gap-2 rounded-[var(--radius-md)] border px-3 py-2 text-left text-[13px]",
                    materials.includes(b.id)
                      ? "border-accent bg-accent-bg text-ink"
                      : "border-line text-ink-soft",
                  )}
                >
                  <Check
                    className={cn(
                      "h-4 w-4 shrink-0",
                      materials.includes(b.id) ? "text-accent" : "opacity-0",
                    )}
                  />
                  <span className="truncate">{b.title}</span>
                </button>
              ))}
            </div>
          )}
        </div>
        <Button block iconLeft={<Play className="h-4 w-4" />} disabled={busy || !goal.trim()} onClick={() => void submit()}>
          {busy ? t("path.generating") : t("path.generate")}
        </Button>
      </div>
    </Sheet>
  );
}

/** 路径详情：横向节点流 + 状态推进 + 手动调整 + AI 评估 + 调整历史 */
function PathDetail({
  pathId,
  onBack,
  onDeleted,
}: {
  pathId: string;
  onBack: () => void;
  onDeleted: () => void;
}) {
  const { t } = useTranslation();
  const [path, setPath] = useState<LearningPath | null>(null);
  const [adjustments, setAdjustments] = useState<PathAdjustment[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [evaluating, setEvaluating] = useState(false);
  const [editing, setEditing] = useState(false);
  const [evalMsg, setEvalMsg] = useState<string | null>(null);

  const books = useLibraryStore((s) => s.books);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const p = await learningPathService.get(pathId);
      setPath(p ?? null);
      setAdjustments(await learningPathService.adjustments(pathId));
      if (books.length === 0) void useLibraryStore.getState().load();
    } catch (e) {
      setError(errMsg(e));
    } finally {
      setLoading(false);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pathId]);

  useEffect(() => {
    void load();
  }, [load]);

  const titleOf = (mid: string | null | undefined) =>
    (mid && books.find((b) => b.id === mid)?.title) || t("path.genericMaterial");

  const setStatus = async (node: PathNode, status: string) => {
    try {
      const p = await learningPathService.nodeStatus(pathId, node.id, status);
      setPath(p);
    } catch (e) {
      toast(errMsg(e));
    }
  };

  const runEvaluate = async () => {
    setEvaluating(true);
    setEvalMsg(null);
    try {
      const r = await learningPathService.adjustEvaluate(pathId);
      if (!r.evaluated) {
        setEvalMsg(t("path.evalNoData"));
      } else if (r.path) {
        setPath((p) => (p ? { ...p, nodes: r.path! } : p));
        setEvalMsg(t("path.evalDone", { count: r.adjustedCount ?? 0 }));
      }
      setAdjustments(await learningPathService.adjustments(pathId));
    } catch (e) {
      toast(errMsg(e));
    } finally {
      setEvaluating(false);
    }
  };

  const removePath = async () => {
    if (!path) return;
    if (!(await askConfirm(t("path.deleteConfirm", { title: path.title })))) return;
    try {
      await learningPathService.remove(pathId);
      onDeleted();
    } catch (e) {
      toast(errMsg(e));
    }
  };

  const title = path?.title || t("path.title");

  if (loading) return <LoadingState />;
  if (error) return <ErrorState message={error} onRetry={() => void load()} />;
  if (!path) return <EmptyState title={t("path.empty")} />;

  return (
    <div className="flex h-full flex-col gap-4 overflow-auto bg-paper px-4 pb-4 pt-3">
      <div>
        <button
          onClick={onBack}
          className="mb-1 inline-flex items-center gap-1 text-sm font-medium text-ink-muted transition hover:text-ink"
        >
          <ArrowLeft className="h-4 w-4" />
          {t("path.backToList")}
        </button>
        <div className="flex items-center justify-between gap-2">
          <h1 className="truncate font-extrabold text-ink" style={{ fontSize: "var(--fs-appbar-h1)" }}>
            {title}
          </h1>
          <div className="flex shrink-0 gap-1">
            <Button size="sm" variant="ghost" iconLeft={<Pencil className="h-4 w-4" />} onClick={() => setEditing((v) => !v)}>
              {t("path.adjustManual")}
            </Button>
            <Button size="sm" variant="ghost" iconLeft={<Trash2 className="h-4 w-4" />} onClick={() => void removePath()}>
              {t("common.delete")}
            </Button>
          </div>
        </div>
        <p className="mt-1 text-sm text-ink-soft">{path.goal}</p>
      </div>

      <Button
        iconLeft={<Crosshair className="h-4 w-4" />}
        disabled={evaluating}
        onClick={() => void runEvaluate()}
      >
        {evaluating ? t("path.evaluating") : t("path.runEvaluate")}
      </Button>
      {evalMsg && (
        <p className="rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2 text-xs text-ink-soft">
          {evalMsg}
        </p>
      )}

      {/* 横向节点流（箭头连接，按 sortOrder） */}
      <Surface pad="none" className="p-4">
        <span className="mb-3 block text-sm font-semibold text-ink">{t("path.nodes")}</span>
        {path.nodes.length === 0 ? (
          <EmptyState title={t("path.noNodes")} />
        ) : (
          <div className="flex items-start gap-1 overflow-x-auto pb-2">
            {path.nodes.map((n, i) => (
              <div key={n.id} className="flex items-start">
                {i > 0 && <ChevronRight className="mt-3 h-4 w-4 shrink-0 text-ink-muted" />}
                <NodeCard
                  node={n}
                  titleOf={titleOf}
                  onStatus={(s) => void setStatus(n, s)}
                />
              </div>
            ))}
          </div>
        )}
      </Surface>

      {/* 手动调整节点 */}
      {editing && (
        <Surface pad="md" className="flex flex-col gap-3">
          <span className="text-sm font-semibold text-ink">{t("path.editNodes")}</span>
          <p className="text-xs text-ink-muted">{t("path.editHint")}</p>
          <NodeEditor
            nodes={path.nodes}
            titleOf={titleOf}
            onSave={async (nodes) => {
              try {
                const p = await learningPathService.update(pathId, nodes);
                setPath(p);
                setEditing(false);
                toast(t("path.updated"));
              } catch (e) {
                toast(errMsg(e));
              }
            }}
          />
        </Surface>
      )}

      {/* AI 调整历史 */}
      <Surface pad="md" className="flex flex-col gap-2">
        <div className="flex items-center gap-2">
          <History className="h-4 w-4 text-ink" />
          <span className="text-sm font-semibold text-ink">{t("path.adjustments")}</span>
        </div>
        {adjustments.length === 0 ? (
          <p className="text-xs text-ink-muted">{t("path.noAdjustments")}</p>
        ) : (
          adjustments.map((a) => (
            <div key={a.id} className="flex items-start justify-between gap-2 border-t border-line pt-2 text-xs">
              <div className="min-w-0">
                <div className="font-semibold text-ink">{a.nodeTitle}</div>
                <div className="text-ink-muted">{a.reason}</div>
              </div>
              <span className="shrink-0 rounded-full bg-accent-bg px-2 py-0.5 text-[10px] font-semibold text-accent">
                {t(`path.action.${a.action}`)}
              </span>
            </div>
          ))
        )}
      </Surface>
    </div>
  );
}

function NodeCard({
  node,
  titleOf,
  onStatus,
}: {
  node: PathNode;
  titleOf: (id: string | null | undefined) => string;
  onStatus: (s: string) => void;
}) {
  const { t } = useTranslation();
  const [menu, setMenu] = useState(false);
  const done = node.status === "completed";
  const Icon = done ? Check : Circle;
  return (
    <div className="w-40 shrink-0">
      <div
        className={cn(
          "relative flex min-h-24 flex-col gap-1 rounded-[var(--radius-md)] border p-3",
          STATUS_STYLE[node.status] ?? STATUS_STYLE.pending,
        )}
      >
        <div className="flex items-start justify-between gap-1">
          <span className="line-clamp-2 text-xs font-semibold leading-tight">{node.title}</span>
        </div>
        <span className="line-clamp-1 text-[10px] opacity-70">{titleOf(node.materialId)}</span>
        <span className="line-clamp-2 text-[10px] opacity-70">{node.goal}</span>
        <div className="mt-auto flex items-center justify-between">
          <span className="rounded-full border border-current/40 px-1.5 py-0.5 text-[9px]">
            {t(`path.status.${node.status}`)}
          </span>
          <button
            type="button"
            onClick={() => setMenu((v) => !v)}
            aria-label={t("path.changeStatus")}
            className="grid h-5 w-5 place-items-center rounded-full border border-line bg-paper text-ink"
          >
            <Icon className="h-3 w-3" />
          </button>
        </div>
        {menu && (
          <div className="absolute right-0 top-full z-10 mt-1 w-36 rounded-[var(--radius-md)] border border-line bg-overlay p-1 shadow-lg">
            {PATH_NODE_STATUS.map((s) => (
              <button
                key={s}
                onClick={() => {
                  onStatus(s);
                  setMenu(false);
                }}
                className="flex w-full items-center gap-2 rounded-[var(--radius-sm)] px-2 py-1.5 text-left text-xs text-overlay hover:bg-overlay-soft"
              >
                {s === node.status && <Check className="h-3.5 w-3.5 text-accent" />}
                {t(`path.status.${s}`)}
              </button>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

/** 手动调整编辑器：增删改 / 排序（learning_path_update 全量替换） */
function NodeEditor({
  nodes,
  titleOf,
  onSave,
}: {
  nodes: PathNode[];
  titleOf: (id: string | null | undefined) => string;
  onSave: (nodes: PathNodeUpdate[]) => Promise<void>;
}) {
  const { t } = useTranslation();
  const [rows, setRows] = useState<PathNodeUpdate[]>(() =>
    nodes.map((n) => ({
      id: n.id,
      materialId: n.materialId,
      title: n.title,
      sortOrder: n.sortOrder,
      goal: n.goal,
      status: n.status,
    })),
  );
  const [busy, setBusy] = useState(false);

  const move = (i: number, dir: -1 | 1) => {
    setRows((r) => {
      const j = i + dir;
      if (j < 0 || j >= r.length) return r;
      const next = [...r];
      [next[i], next[j]] = [next[j], next[i]];
      return next.map((row, k) => ({ ...row, sortOrder: k }));
    });
  };

  const updateRow = (i: number, patch: Partial<PathNodeUpdate>) =>
    setRows((r) => r.map((row, k) => (k === i ? { ...row, ...patch } : row)));

  const removeRow = (i: number) => setRows((r) => r.filter((_, k) => k !== i));
  const addRow = () =>
    setRows((r) => [
      ...r,
      {
        id: null,
        materialId: null,
        title: "",
        goal: "",
        sortOrder: r.length,
        status: "pending",
      },
    ]);

  return (
    <div className="flex flex-col gap-2">
      {rows.map((row, i) => (
        <div key={row.id ?? `new-${i}`} className="flex flex-col gap-2 rounded-[var(--radius-md)] border border-line p-3">
          <div className="flex items-center gap-2">
            <button type="button" onClick={() => move(i, -1)} className="rounded border border-line px-2 py-0.5 text-xs text-ink">↑</button>
            <button type="button" onClick={() => move(i, 1)} className="rounded border border-line px-2 py-0.5 text-xs text-ink">↓</button>
            <span className="flex-1 text-[10px] text-ink-muted">{titleOf(row.materialId)}</span>
            <button type="button" onClick={() => removeRow(i)} className="rounded p-1 text-danger hover:bg-paper-soft" aria-label={t("common.delete")}>
              <X className="h-4 w-4" />
            </button>
          </div>
          <input
            value={row.title}
            onChange={(e) => updateRow(i, { title: e.target.value })}
            placeholder={t("path.nodeTitlePlaceholder")}
            className="h-9 rounded-[var(--radius-md)] border border-line bg-paper px-2 text-sm text-ink outline-none focus:border-accent"
          />
          <input
            value={row.goal}
            onChange={(e) => updateRow(i, { goal: e.target.value })}
            placeholder={t("path.nodeGoalPlaceholder")}
            className="h-9 rounded-[var(--radius-md)] border border-line bg-paper px-2 text-sm text-ink outline-none focus:border-accent"
          />
        </div>
      ))}
      <Button size="sm" variant="secondary" iconLeft={<Plus className="h-4 w-4" />} onClick={addRow}>
        {t("path.addNode")}
      </Button>
      <Button
        variant="primary"
        disabled={busy || rows.some((r) => !r.title.trim())}
        onClick={() => {
          setBusy(true);
          void onSave(rows.map((r) => ({ ...r, sortOrder: r.sortOrder ?? 0 })) as PathNodeUpdate[])
            .catch((e) => toast(errMsg(e)))
            .finally(() => setBusy(false));
        }}
      >
        {busy ? <Loader2 className="h-4 w-4 animate-spin" /> : t("path.saveChanges")}
      </Button>
    </div>
  );
}