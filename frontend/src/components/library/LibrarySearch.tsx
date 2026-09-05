import { useTranslation } from "react-i18next";
import { Search } from "lucide-react";
import { useLibraryStore } from "../../stores/libraryStore";

/** 书架搜索栏：书名 / 作者 / 标签 */
export function LibrarySearch() {
  const { t } = useTranslation();
  const query = useLibraryStore((s) => s.query);
  const setQuery = useLibraryStore((s) => s.setQuery);

  return (
    <div className="flex items-center gap-2 rounded-[var(--radius-md)] border border-line bg-paper-soft px-3 py-2">
      <Search className="h-4 w-4 text-ink-muted" />
      <input
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        placeholder={t("library.searchPlaceholder")}
        className="flex-1 bg-transparent text-sm text-ink outline-none placeholder:text-ink-muted"
      />
    </div>
  );
}
