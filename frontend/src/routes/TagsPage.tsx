import { useCallback, useEffect, useMemo, useState } from "react";
import { askConfirm } from "../components/ui/confirmService";
import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { ChevronRight, Plus, Search, Tags, X, Pencil } from "lucide-react";
import { Surface } from "../components/ui/Surface";
import { Button } from "../components/ui/Button";
import { EmptyState } from "../components/common/states";
import { SubBackHeader } from "../components/shell/SubBackHeader";
import { cn } from "../utils/cn";
import { logError } from "../utils/logError";
import {
  tagService,
  TAG_COLOR_PALETTE,
  type TagNode,
  type TagScope,
} from "../services/tagService";

/** 打标作用域选项 */
const SCOPES: TagScope[] = [
  "book",
  "highlight",
  "note",
  "knowledge",
  "card",
  "misquestion",
  "whiteCard",
];

export function TagsPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();

  // 标签树
  const [tree, setTree] = useState<TagNode[]>([]);
  const [keyword, setKeyword] = useState("");
  const [searchResults, setSearchResults] = useState<TagNode[]>([]);
  const [loading, setLoading] = useState(true);

  // 新建标签
  const [newName, setNewName] = useState("");
  const [newParent, setNewParent] = useState("");
  const [newColor, setNewColor] = useState(TAG_COLOR_PALETTE[0]);

  // 重命名
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editName, setEditName] = useState("");

  // 打标面板
  const [scope, setScope] = useState<TagScope>("book");
  const [scopeId, setScopeId] = useState("");
  const [applied, setApplied] = useState<TagNode[]>([]);
  const [applyInput, setApplyInput] = useState("");

  const loadTree = useCallback(async () => {
    setLoading(true);
    try {
      setTree(await tagService.getTree());
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadTree();
  }, [loadTree]);

  const allTags = useMemo(() => {
    const out: TagNode[] = [];
    const walk = (nodes: TagNode[]) => {
      for (const n of nodes) {
        out.push(n);
        walk(n.children);
      }
    };
    walk(tree);
    return out;
  }, [tree]);

  const createTag = async () => {
    const name = newName.trim();
    if (!name) return;
    try {
      await tagService.create(name, newParent || null, newColor);
      setNewName("");
      setNewParent("");
      await loadTree();
    } catch (e) {
      // 后端已返回 AppError 文案（重复/超长等），此处仅留痕避免静默
      logError("TagsPage.createTag", e);
    }
  };

  const startRename = (node: TagNode) => {
    setEditingId(node.id);
    setEditName(node.name);
  };

  const commitRename = async (id: string) => {
    const name = editName.trim();
    if (!name) return;
    try {
      await tagService.rename(id, name);
    } finally {
      setEditingId(null);
      await loadTree();
    }
  };

  const deleteTag = async (node: TagNode) => {
    // 无合并目标，直接删除（子标签 parent_id 由后端自动置空）
    if (!(await askConfirm(t("tags.deleteConfirm", { name: node.name })))) return;
    try {
      await tagService.delete(node.id, null);
      await loadTree();
    } catch (e) {
      // 后端错误文案已归一化，此处仅留痕
      logError("TagsPage.deleteTag", e);
    }
  };

  const runSearch = useCallback(async (kw: string) => {
    if (!kw.trim()) {
      setSearchResults([]);
      return;
    }
    setSearchResults(await tagService.search(kw));
  }, []);

  // 打标
  const loadApplied = async () => {
    if (!scopeId.trim()) return;
    setApplied(await tagService.listFor(scope, scopeId.trim()));
  };

  const applyTag = async () => {
    const name = applyInput.trim();
    if (!name || !scopeId.trim()) return;
    try {
      await tagService.apply(scope, scopeId.trim(), [name], false);
      setApplyInput("");
      await loadApplied();
    } catch (e) {
      // 后端错误文案，此处仅留痕
      logError("TagsPage.applyTag", e);
    }
  };

  const removeTag = async (tagId: string) => {
    if (!scopeId.trim()) return;
    try {
      await tagService.remove(scope, scopeId.trim(), tagId);
      await loadApplied();
    } catch (e) {
      // 后端错误文案，此处仅留痕
      logError("TagsPage.removeTag", e);
    }
  };

  const showSearch = keyword.trim().length > 0;

  return (
    <div className="flex h-full flex-col overflow-auto bg-paper pb-4 pt-0">
      <SubBackHeader titleKey="tags.title" onBack={() => navigate(-1)} />
      <div className="flex flex-col gap-4 px-4 pt-3">
        <div className="flex items-center gap-1 self-start rounded-full border border-line bg-paper-soft px-2.5 py-1">
          <Search className="h-4 w-4 text-ink-muted" />
          <input
            value={keyword}
            onChange={(e) => {
              setKeyword(e.target.value);
              void runSearch(e.target.value);
            }}
            placeholder={t("tags.searchPlaceholder")}
            className="w-32 bg-transparent text-sm text-ink outline-none placeholder:text-ink-muted"
          />
        </div>

      {/* 新建标签 */}
      <Surface pad="md" className="flex flex-col gap-2">
        <div className="flex items-center gap-2 text-sm font-semibold text-ink">
          <Plus className="h-4 w-4" />
          {t("tags.create")}
        </div>
        <div className="grid grid-cols-1 gap-2 sm:grid-cols-4">
          <input
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            placeholder={t("tags.name")}
            className="rounded-[var(--radius-md)] border border-line bg-paper px-3 py-2 text-sm text-ink outline-none placeholder:text-ink-muted"
          />
          <select
            value={newParent}
            onChange={(e) => setNewParent(e.target.value)}
            className="rounded-[var(--radius-md)] border border-line bg-paper px-3 py-2 text-sm text-ink outline-none"
          >
            <option value="">{t("tags.root")}</option>
            {allTags.map((tg) => (
              <option key={tg.id} value={tg.id}>
                {tg.name}
              </option>
            ))}
          </select>
          <div className="flex items-center gap-1.5 px-1">
            {TAG_COLOR_PALETTE.map((c) => (
              <button
                key={c}
                onClick={() => setNewColor(c)}
                className={cn(
                  "h-5 w-5 rounded-full border transition",
                  newColor === c ? "border-accent ring-2 ring-accent/30" : "border-line",
                )}
                style={{ backgroundColor: c }}
                aria-label={t("tags.color")}
              />
            ))}
          </div>
          <Button size="sm" onClick={() => void createTag()}>
            {t("tags.createSubmit")}
          </Button>
        </div>
      </Surface>

      {/* 标签树 / 搜索结果 */}
      <Surface pad="none" className="overflow-hidden">
        <div className="border-b border-line px-4 py-2.5 text-sm font-semibold text-ink">
          {showSearch ? t("tags.searchResult") : t("tags.tree")}
        </div>
        {loading ? (
          <div className="px-4 py-6 text-sm text-ink-muted">{t("common.loading")}</div>
        ) : showSearch ? (
          <div className="flex flex-col">
            {searchResults.length === 0 ? (
              <EmptyState title={t("tags.empty")} />
            ) : (
              searchResults.map((n) => <TagRow key={n.id} node={n} />)
            )}
          </div>
        ) : tree.length === 0 ? (
          <EmptyState title={t("tags.empty")} />
        ) : (
          <div className="flex flex-col">
            {tree.map((n) => (
              <TagNodeItem
                key={n.id}
                node={n}
                depth={0}
                editingId={editingId}
                editName={editName}
                onEditName={setEditName}
                onStartRename={(node) => {
                  startRename(node);
                }}
                onCommitRename={(id) => void commitRename(id)}
                onCancelRename={() => setEditingId(null)}
                onDelete={(node) => void deleteTag(node)}
              />
            ))}
          </div>
        )}
      </Surface>

      {/* 给内容打标面板 */}
      <Surface pad="md" className="flex flex-col gap-2">
        <div className="flex items-center gap-2 text-sm font-semibold text-ink">
          <Tags className="h-4 w-4" />
          {t("tags.tagContent")}
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <select
            value={scope}
            onChange={(e) => setScope(e.target.value as TagScope)}
            className="rounded-[var(--radius-md)] border border-line bg-paper px-3 py-2 text-sm text-ink outline-none"
          >
            {SCOPES.map((s) => (
              <option key={s} value={s}>
                {s}
              </option>
            ))}
          </select>
          <input
            value={scopeId}
            onChange={(e) => setScopeId(e.target.value)}
            placeholder={t("tags.scopeId")}
            className="min-w-40 flex-1 rounded-[var(--radius-md)] border border-line bg-paper px-3 py-2 text-sm text-ink outline-none placeholder:text-ink-muted"
          />
          <Button size="sm" variant="secondary" onClick={() => void loadApplied()}>
            {t("tags.check")}
          </Button>
        </div>

        {applied.length > 0 ? (
          <div className="flex flex-wrap items-center gap-1.5">
            {applied.map((tag) => (
              <span
                key={tag.id}
                className="inline-flex items-center gap-1.5 rounded-full border border-line bg-paper-soft px-2.5 py-1 text-xs text-ink"
              >
                <span className="h-2.5 w-2.5 rounded-full" style={{ backgroundColor: tag.color }} />
                {tag.name}
                <button
                  onClick={() => void removeTag(tag.id)}
                  className="text-ink-muted hover:text-danger"
                  aria-label={t("tags.remove")}
                >
                  <X className="h-3.5 w-3.5" />
                </button>
              </span>
            ))}
          </div>
        ) : (
          scopeId.trim() && (
            <p className="text-xs text-ink-muted">{t("tags.noApplied")}</p>
          )
        )}

        <div className="flex items-center gap-2">
          <input
            value={applyInput}
            onChange={(e) => setApplyInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void applyTag();
            }}
            placeholder={t("tags.tagNames")}
            className="flex-1 rounded-[var(--radius-md)] border border-line bg-paper px-3 py-2 text-sm text-ink outline-none placeholder:text-ink-muted"
          />
          <Button size="sm" onClick={() => void applyTag()}>
            {t("tags.apply")}
          </Button>
        </div>
      </Surface>
      </div>
    </div>
  );
}

/** 树中单行标签（含折叠展开） */
function TagNodeItem(props: {
  node: TagNode;
  depth: number;
  editingId: string | null;
  editName: string;
  onEditName: (v: string) => void;
  onStartRename: (node: TagNode) => void;
  onCommitRename: (id: string) => void;
  onCancelRename: () => void;
  onDelete: (node: TagNode) => void;
}) {
  const { t } = useTranslation();
  const { node, depth } = props;
  const isEditing = props.editingId === node.id;
  const [open, setOpen] = useState(true);

  return (
    <div>
      <div
        className="flex items-center justify-between gap-2 border-b border-line px-3 py-2"
        style={{ paddingLeft: 12 + depth * 20 }}
      >
        <div className="flex min-w-0 items-center gap-1.5">
          {node.children.length > 0 ? (
            <button onClick={() => setOpen((o) => !o)} className="shrink-0 text-ink-muted">
              <ChevronRight className={cn("h-3.5 w-3.5 transition", open && "rotate-90")} />
            </button>
          ) : (
            <span className="w-3.5 shrink-0" />
          )}
          <span className="h-2.5 w-2.5 shrink-0 rounded-full" style={{ backgroundColor: node.color }} />
          {isEditing ? (
            <input
              autoFocus
              value={props.editName}
              onChange={(e) => props.onEditName(e.target.value)}
              onBlur={() => props.onCommitRename(node.id)}
              onKeyDown={(e) => {
                if (e.key === "Enter") props.onCommitRename(node.id);
                if (e.key === "Escape") props.onCancelRename();
              }}
              className="min-w-0 flex-1 rounded border border-line bg-paper px-2 py-0.5 text-sm text-ink outline-none"
            />
          ) : (
            <span className="truncate text-sm text-ink">{node.name}</span>
          )}
        </div>
        <div className="flex shrink-0 items-center gap-1">
          <button
            onClick={() => props.onStartRename(node)}
            className="p-1 text-ink-muted hover:text-ink"
            aria-label={t("tags.rename")}
          >
            <Pencil className="h-3.5 w-3.5" />
          </button>
          <button
            onClick={() => props.onDelete(node)}
            className="p-1 text-ink-muted hover:text-danger"
            aria-label={t("tags.delete")}
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>
      </div>
      {open &&
        node.children.map((c) => <TagNodeItem key={c.id} {...props} node={c} depth={depth + 1} />)}
    </div>
  );
}

/** 搜索结果单行（快捷重命名/删除） */
function TagRow({ node }: { node: TagNode }) {
  const { t } = useTranslation();
  const deleteTag = async () => {
    if (!(await askConfirm(t("tags.deleteConfirm", { name: node.name })))) return;
    await tagService.delete(node.id, null);
  };
  return (
    <div className="flex items-center justify-between gap-2 border-b border-line px-3 py-2">
      <div className="flex min-w-0 items-center gap-1.5">
        <span className="h-2.5 w-2.5 shrink-0 rounded-full" style={{ backgroundColor: node.color }} />
        <span className="truncate text-sm text-ink">{node.name}</span>
      </div>
      <button onClick={() => void deleteTag()} className="p-1 text-ink-muted hover:text-danger" aria-label={t("tags.delete")}>
        <X className="h-3.5 w-3.5" />
      </button>
    </div>
  );
}