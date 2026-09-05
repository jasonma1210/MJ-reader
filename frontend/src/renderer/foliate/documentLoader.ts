/**
 * foliate-js 文档加载器（移植自 frontend-deprecated）。
 *
 * 构造带文件名的真 File 交给 view.open()，并自己按格式调用 EPUB / MOBI / FB2 /
 * ComicBook 解析器，把已解析的 book 对象交给 view.open()，彻底绕开 foliate 脆弱的
 * 格式嗅探（Blob 无 name → undefined.endsWith 崩溃）。详见 deprecated 注释。
 */

import { logError } from "../../utils/logError";
import i18n from "../../i18n";

/** 已解析的 foliate book 对象（结构由各解析器决定，这里只做透传） */
export type FoliateBook = Record<string, unknown>;

export interface LoadedFoliateBook {
  /** 交给 view.open() 的已解析 book 对象 */
  book: FoliateBook;
  /** 实际识别出的格式（大写），可能与文件后缀不一致 */
  detectedFormat: string;
}

/** 后缀 → MIME。文件名与 MIME 双保险，任一命中 foliate 都能正确分流 */
const EXT_MIME: Record<string, string> = {
  epub: "application/epub+zip",
  mobi: "application/x-mobipocket-ebook",
  azw: "application/x-mobipocket-ebook",
  azw3: "application/x-mobipocket-ebook",
  prc: "application/x-mobipocket-ebook",
  fb2: "application/x-fictionbook+xml",
  fbz: "application/x-zip-compressed-fb2",
  cbz: "application/vnd.comicbook+zip",
  zip: "application/zip",
};

/** 从路径里取文件名；content:// URI 与 Windows 反斜杠都兼容 */
function basename(filePath: string, fallbackExt: string): string {
  const cleaned = filePath.split(/[?#]/)[0] ?? filePath;
  const segments = cleaned.split(/[/\\]/);
  const last = segments[segments.length - 1] ?? "";
  const decoded = (() => {
    try {
      return decodeURIComponent(last);
    } catch (e) {
      logError("renderer/foliate/documentLoader.basename.decode", e);
      return last;
    }
  })();
  if (!decoded || !decoded.includes(".")) {
    return `book.${fallbackExt || "epub"}`;
  }
  return decoded;
}

async function hasZipMagic(file: File): Promise<boolean> {
  if (file.size < 4) return false;
  const head = new Uint8Array(await file.slice(0, 4).arrayBuffer());
  return head[0] === 0x50 && head[1] === 0x4b && head[2] === 0x03;
}

async function hasEOCD(file: File): Promise<boolean> {
  const MAX_COMMENT = 64 * 1024;
  const EOCD_SIZE = 22;
  const sliceSize = Math.min(MAX_COMMENT + EOCD_SIZE, file.size);
  if (sliceSize < EOCD_SIZE) return false;
  const tail = new Uint8Array(
    await file.slice(file.size - sliceSize, file.size).arrayBuffer(),
  );
  for (let i = tail.length - EOCD_SIZE; i >= 0; i--) {
    if (
      tail[i] === 0x50 &&
      tail[i + 1] === 0x4b &&
      tail[i + 2] === 0x05 &&
      tail[i + 3] === 0x06
    ) {
      return true;
    }
  }
  return false;
}

async function isZipFile(file: File): Promise<boolean> {
  if (await hasZipMagic(file)) return true;
  return hasEOCD(file);
}

const isCBZ = (file: File): boolean =>
  file.type === "application/vnd.comicbook+zip" ||
  file.name.toLowerCase().endsWith(".cbz");

const isFB2 = (file: File): boolean =>
  file.type === "application/x-fictionbook+xml" ||
  file.name.toLowerCase().endsWith(".fb2");

const isFBZ = (file: File): boolean =>
  file.type === "application/x-zip-compressed-fb2" ||
  file.name.toLowerCase().endsWith(".fb2.zip") ||
  file.name.toLowerCase().endsWith(".fbz");

async function makeZipLoader(file: File) {
  const { configure, ZipReader, BlobReader, TextWriter, BlobWriter } =
    await import("foliate-js/vendor/zip.js");
  // Android WebView 里 Web Worker 加载 blob: 脚本受限，关掉更稳
  configure({ useWebWorkers: false });
  const reader = new ZipReader(new BlobReader(file));
  const entries = await reader.getEntries();
  const map = new Map(entries.map((entry) => [entry.filename, entry]));
  const loadText = (name: string): Promise<string> | null => {
    const entry = map.get(name);
    return entry
      ? (entry.getData(new TextWriter()) as unknown as Promise<string>)
      : null;
  };
  const loadBlob = (name: string, type?: string): Promise<Blob> | null => {
    const entry = map.get(name);
    return entry
      ? (entry.getData(new BlobWriter(type)) as unknown as Promise<Blob>)
      : null;
  };
  const getSize = (name: string): number =>
    map.get(name)?.uncompressedSize ?? 0;
  return { entries, loadText, loadBlob, getSize };
}

/** 把后端读到的字节包成带文件名的真 File —— 修复 `.endsWith` 崩溃的关键一步 */
export function bytesToFile(
  bytes: Uint8Array,
  filePath: string,
  format: string,
): File {
  const fmt = (format || "").toLowerCase();
  const name = basename(filePath, fmt);
  const type = EXT_MIME[fmt] ?? "application/octet-stream";
  const buffer = bytes.buffer.slice(
    bytes.byteOffset,
    bytes.byteOffset + bytes.byteLength,
  ) as ArrayBuffer;
  return new File([buffer], name, { type });
}

/**
 * 解析电子书为 foliate book 对象。
 * @param bytes  后端 read_file_bytes 读到的原始字节
 * @param filePath 原始路径（仅用于取文件名与诊断）
 * @param format 书库里记录的格式（epub/mobi/azw3/...）
 */
export async function openFoliateBook(
  bytes: Uint8Array,
  filePath: string,
  format: string,
): Promise<LoadedFoliateBook> {
  const file = bytesToFile(bytes, filePath, format);
  if (!file.size) {
    throw new Error(i18n.t("reader.loadEmptyFile"));
  }

  const ext = (file.name.split(".").pop() ?? "").toLowerCase();
  let book: unknown = null;
  let detectedFormat = "";

  if (await isZipFile(file)) {
    let loader: Awaited<ReturnType<typeof makeZipLoader>>;
    try {
      loader = await makeZipLoader(file);
    } catch (e) {
      logError("renderer/foliate/documentLoader.makeZipLoader", e);
      throw new Error(i18n.t("reader.loadZipExtract", { name: file.name }));
    }

    if (isCBZ(file)) {
      const { makeComicBook } = await import("foliate-js/comic-book.js");
      book = makeComicBook(loader, file);
      detectedFormat = "CBZ";
    } else if (isFBZ(file)) {
      const { makeFB2 } = await import("foliate-js/fb2.js");
      const entry =
        loader.entries.find((it) => it.filename.toLowerCase().endsWith(".fb2")) ??
        loader.entries[0];
      if (!entry) throw new Error(i18n.t("reader.loadFbzEmpty"));
      const blob = await loader.loadBlob(entry.filename);
      if (!blob) throw new Error(i18n.t("reader.loadFbzReadFail", { name: entry.filename }));
      book = await makeFB2(blob);
      detectedFormat = "FB2";
    } else {
      const hasContainer = loader.entries.some(
        (it) => it.filename === "META-INF/container.xml",
      );
      if (!hasContainer) {
        throw new Error(i18n.t("reader.loadNoContainer"));
      }
      const { EPUB } = await import("foliate-js/epub.js");
      book = await new EPUB(loader).init();
      detectedFormat = "EPUB";
    }
  } else {
    const { isMOBI, MOBI } = await import("foliate-js/mobi.js");
    if (await isMOBI(file)) {
      const { unzlibSync } = await import("foliate-js/vendor/fflate.js");
      try {
        book = await new MOBI({ unzlib: unzlibSync }).open(file);
      } catch (e) {
        logError("renderer/foliate/documentLoader.mobi", e);
        throw new Error(i18n.t("reader.loadZipExtract", { name: file.name }));
      }
      detectedFormat =
        ext === "azw3" ? "AZW3" : ext === "azw" ? "AZW" : "MOBI";
    } else if (isFB2(file)) {
      const { makeFB2 } = await import("foliate-js/fb2.js");
      book = await makeFB2(file);
      detectedFormat = "FB2";
    }
  }

  // MOBI/AZW/PRC 强制走 reflowable（连续流式排版），避免固定版式在窄视口只露一小块
  if (book && ["mobi", "azw", "azw3", "prc"].includes(ext)) {
    const b = book as { rendition?: Record<string, unknown> };
    b.rendition = { ...(b.rendition ?? {}), layout: "reflowable" };
  }

  if (!book) {
    throw new Error(i18n.t("reader.loadUnrecognized", { name: file.name }));
  }

  return { book: book as FoliateBook, detectedFormat };
}
