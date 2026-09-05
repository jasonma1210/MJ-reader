import { lazy, Suspense, useMemo } from "react";
import { Loader2 } from "lucide-react";
import { FoliateView } from "./foliate/FoliateView";
import type { OfficeFormat } from "./office/OfficeView";
import type { TextMode } from "./text/TextView";

// C2（bundle 分包）：PDF/Office/Text 渲染器懒加载 —— pdfjs-dist / xlsx / pptx-renderer
// 不再打进主 chunk，仅打开对应格式时按需加载（主 bundle 从 1.38MB 显著下降）。
const PdfView = lazy(() => import("./pdf/PdfView").then((m) => ({ default: m.PdfView })));
const OfficeViewLazy = lazy(() =>
  import("./office/OfficeView").then((m) => ({ default: m.OfficeView })),
);
const TextViewLazy = lazy(() =>
  import("./text/TextView").then((m) => ({ default: m.TextView })),
);

const OFFICE_FORMATS = new Set<OfficeFormat>([
  "docx", "doc", "pptx", "ppt", "xlsx", "xls", "rtf", "odt", "ods", "odp",
]);
const TEXT_FORMATS = new Set<string>(["txt", "md", "markdown", "html", "htm", "xhtml", "xht", "xml", "mhtml", "mht", "mhtm"]);

function LazyFallback() {
  return (
    <div className="flex h-full w-full items-center justify-center bg-paper">
      <Loader2 className="h-6 w-6 animate-spin text-accent" />
    </div>
  );
}

/**
 * 统一书籍渲染调度（对齐 deprecated ReaderRenderer 的 13+ 格式分发）：
 * - pdf → PdfView（pdfjs + 拼音修复，懒加载）
 * - docx/doc/pptx/ppt/xlsx/xls/rtf/odt/ods/odp → OfficeView（懒加载）
 * - txt/md/html/xml/mhtml → TextView（懒加载）
 * - epub/mobi/azw/azw3/fb2/cbz/zip → FoliateView
 */
export function BookView({
  bookId,
  bookPath,
  format,
}: {
  bookId: string;
  bookPath: string;
  format: string;
}) {
  const fmt = (format || "").toLowerCase();

  const view = useMemo(() => {
    if (fmt === "pdf") {
      return (
        <Suspense fallback={<LazyFallback />}>
          <PdfView bookId={bookId} bookPath={bookPath} />
        </Suspense>
      );
    }
    if (OFFICE_FORMATS.has(fmt as OfficeFormat)) {
      return (
        <Suspense fallback={<LazyFallback />}>
          <OfficeViewLazy bookId={bookId} bookPath={bookPath} format={fmt as OfficeFormat} />
        </Suspense>
      );
    }
    if (TEXT_FORMATS.has(fmt)) {
      const mode =
        fmt === "markdown"
          ? "md"
          : fmt === "htm" || fmt === "xhtml" || fmt === "xht"
            ? "html"
            : fmt === "mht" || fmt === "mhtm"
              ? "mhtml"
              : (fmt as TextMode);
      return (
        <Suspense fallback={<LazyFallback />}>
          <TextViewLazy bookId={bookId} bookPath={bookPath} mode={mode} />
        </Suspense>
      );
    }
    return <FoliateView bookId={bookId} />;
  }, [fmt, bookId, bookPath]);

  return view;
}
