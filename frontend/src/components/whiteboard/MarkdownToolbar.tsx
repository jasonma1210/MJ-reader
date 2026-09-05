import { useTranslation } from "react-i18next";
import { Bold, Code, Heading1, Italic, Link2, List } from "lucide-react";

/**
 * Req4「钉一钉」富文本 / 思维导图卡片的轻量 Markdown 格式工具栏：
 * 在 textarea 光标处插入 md 语法（包裹选区 / 行首前缀），并同步受控组件的值。
 *
 * 用法：
 *   const ref = useRef<HTMLTextAreaElement>(null);
 *   <MarkdownToolbar targetRef={ref} getValue={() => md} setValue={setMd} />
 *   <textarea ref={ref} value={md} onChange={(e) => setMd(e.target.value)} />
 */
export function MarkdownToolbar({
  targetRef,
  getValue,
  setValue,
}: {
  targetRef: React.RefObject<HTMLTextAreaElement | null>;
  getValue: () => string;
  setValue: (v: string) => void;
}) {
  const { t } = useTranslation();

  /** 在光标/选区处应用 markdown 语法，更新受控值并恢复选区焦点 */
  const apply = (kind: "bold" | "italic" | "code" | "link" | "heading" | "list") => {
    const ta = targetRef.current;
    if (!ta) return;
    const val = getValue();
    const s = ta.selectionStart;
    const e = ta.selectionEnd;
    let next = val;
    let selStart = s;
    let selEnd = e;

    const wrap = (before: string, after: string, placeholder: string) => {
      const sel = val.slice(s, e);
      const content = sel || placeholder;
      next = val.slice(0, s) + before + content + after + val.slice(e);
      selStart = s + before.length;
      selEnd = selStart + content.length;
    };
    const prefixLine = (prefix: string) => {
      // 多行选区逐行加前缀；无选区则在当前行首插入
      const lineStart = val.lastIndexOf("\n", s - 1) + 1;
      const lineEndIdx = val.indexOf("\n", e);
      const lineEnd = lineEndIdx === -1 ? val.length : lineEndIdx;
      const block = val.slice(lineStart, lineEnd);
      const lines = block
        .split("\n")
        .map((l) => (l.startsWith(prefix.trim()) ? l : prefix + l))
        .join("\n");
      next = val.slice(0, lineStart) + lines + val.slice(lineEnd);
      selStart = lineStart;
      selEnd = lineStart + lines.length;
    };

    switch (kind) {
      case "bold":
        wrap("**", "**", t("whiteboard.pin.mdBold"));
        break;
      case "italic":
        wrap("*", "*", t("whiteboard.pin.mdItalic"));
        break;
      case "code":
        wrap("`", "`", t("whiteboard.pin.mdCode"));
        break;
      case "link":
        wrap("[", "](https://)", t("whiteboard.pin.mdLink"));
        break;
      case "heading":
        prefixLine("# ");
        break;
      case "list":
        prefixLine("- ");
        break;
    }
    setValue(next);
    // 等受控组件重渲染后再恢复选区与焦点
    requestAnimationFrame(() => {
      ta.focus();
      ta.setSelectionRange(selStart, selEnd);
    });
  };

  const btns: { kind: Parameters<typeof apply>[0]; icon: React.ReactNode; label: string }[] = [
    { kind: "heading", icon: <Heading1 className="h-3.5 w-3.5" />, label: t("whiteboard.pin.mdHeading") },
    { kind: "bold", icon: <Bold className="h-3.5 w-3.5" />, label: t("whiteboard.pin.mdBoldBtn") },
    { kind: "italic", icon: <Italic className="h-3.5 w-3.5" />, label: t("whiteboard.pin.mdItalicBtn") },
    { kind: "list", icon: <List className="h-3.5 w-3.5" />, label: t("whiteboard.pin.mdList") },
    { kind: "code", icon: <Code className="h-3.5 w-3.5" />, label: t("whiteboard.pin.mdCodeBtn") },
    { kind: "link", icon: <Link2 className="h-3.5 w-3.5" />, label: t("whiteboard.pin.mdLinkBtn") },
  ];

  return (
    <div className="mb-1.5 flex items-center gap-0.5 rounded-[var(--radius-md)] border border-line bg-paper-soft p-0.5">
      {btns.map((b) => (
        <button
          key={b.kind}
          type="button"
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => apply(b.kind)}
          title={b.label}
          aria-label={b.label}
          className="rounded-[var(--radius-sm)] p-1 text-ink-muted transition hover:bg-paper active:bg-line"
        >
          {b.icon}
        </button>
      ))}
    </div>
  );
}
