import { CMD, invoke, isTauri } from "./tauri";
import type { ImportTask } from "../types";
import { logError } from "../utils/logError";

/** Android content:// URI 元数据（Tauri 序列化为 camelCase：displayName / size / mimeType） */
export interface ContentUriMeta {
  displayName: string;
  size: number;
  mimeType: string;
}

/** 后端 import-progress / import-done / import-error 事件载荷（camelCase） */
export interface ImportStatusEvent {
  id: string;
  stage: string;
  percent: number;
  message: string;
  fileName?: string | null;
  book?: unknown;
  error?: string | null;
}

/** 解析 Android content:// URI 的原始文件名（避免显示 document%xxxx / document_4614） */
export async function resolveContentUriName(path: string): Promise<string | null> {
  if (!isTauri() || !path.startsWith("content://")) return null;
  // 1) 首选：Rust get_content_uri_metadata（ContentResolver 查 DISPLAY_NAME，权威）
  //    即便返回 document_4614 也保留（那就是真实文件名），由文件元数据回填真实书名。
  try {
    const meta = await invoke<ContentUriMeta>(CMD.getContentUriMetadata, { uri: path });
    const name = meta.displayName?.trim();
    if (name) return name;
  } catch (e) {
  logError("importService.name", e);
  }
  // 2) 兜底：plugin-fs stat（部分 ROM 的 SAF URI 该通道可拿到文件名）
  try {
    const { stat } = await import("@tauri-apps/plugin-fs");
    const info = await stat(path);
    const name = (info as { name?: string })?.name?.trim();
    if (name) return name;
  } catch (e) {
  logError("importService.name", e);
  }
  return null;
}

/**
 * 从 URI/路径提取上传文件名（按用户裁定：无元数据时直接用上传文件名）。
 * - 只解码 + 去掉 content:// 方案前缀与 primary: 文件路径前缀（那是 URI 结构，不是名字）；
 * - 不剥 document_ 前缀——document_4614 就是文件的真实名字，应原样使用。
 */
function fallbackName(path: string): string {
  let seg = path.split(/[?#]/)[0] ?? path;
  if (/^content:/i.test(seg)) {
    // content://.../document/<name> 或 /tree/<name>/document/<name>
    const m = /\/document\/([^/]+)$/.exec(seg);
    seg = m ? m[1] : (seg.split(/[/\\]/).pop() ?? "");
  } else {
    seg = seg.split(/[/\\]/).pop() ?? "";
  }
  try {
    seg = decodeURIComponent(seg);
  } catch (e) {
  logError("importService.m", e);
  }
  // 去掉 primary: / primary%2F 这类存储根前缀（URI 内部路径，非文件名）
  seg = seg.replace(/^primary[:/]/i, "");
  return seg || path;
}

/** 规范化路径：content:// 原样保留；file:// 还原为本地绝对路径 */
function normalizePath(path: string): string {
  if (/^content:\/\//i.test(path) || path.startsWith("/")) return path;
  if (/^file:\/\//i.test(path)) {
    let p = decodeURIComponent(path.replace(/^file:\/\//i, ""));
    if (!p.startsWith("/")) p = "/" + p;
    return p;
  }
  return path;
}

/**
 * 把 content:// 临时复制到书库目录（SAF 直读失败的 ROM 兜底通道）。
 * 使用 tauri-plugin-fs 读取字节 → 写入 app_data/documents → start_import_book。
 */
async function fallbackImportViaTempFile(
  contentUri: string,
  displayName: string | null,
): Promise<string> {
  const [{ readFile, writeFile, mkdir }, { appDataDir, join }] = await Promise.all([
    import("@tauri-apps/plugin-fs"),
    import("@tauri-apps/api/path"),
  ]);
  const bytes = await readFile(contentUri);
  const base = await appDataDir();
  const dir = await join(base, "documents");
  await mkdir(dir, { recursive: true }).catch(() => {
    /* 目录已存在 */
  });
  const safeName = (displayName || fallbackName(contentUri) || "unknown_file").replace(
    /[\\/:*?"<>|]/g,
    "_",
  );
  const hasOwnExt = /\.[a-z0-9]+$/i.test(safeName);
  const fileName = `${crypto.randomUUID()}__${hasOwnExt ? safeName : `${safeName}.${guessExt(safeName)}`}`;
  const tempPath = await join(dir, fileName);
  await writeFile(tempPath, bytes);
  return invoke<string>(CMD.startImportBook, {
    filePath: tempPath,
    displayName: displayName || null,
  });
}

/** 按后缀挑一个扩展名用于临时文件命名（后端会再做字节嗅探） */
const EXT_HINTS: Array<[RegExp, string]> = [
  [/\.epub$/i, "epub"], [/\.pdf$/i, "pdf"], [/\.mobi$/i, "mobi"],
  [/\.azw3?$/i, "azw3"], [/\.fb2$/i, "fb2"], [/\.cbz$/i, "cbz"],
  [/\.zip$/i, "zip"], [/\.txt$/i, "txt"], [/\.md$/i, "md"],
  [/\.docx$/i, "docx"], [/\.pptx$/i, "pptx"], [/\.xlsx$/i, "xlsx"],
];
function guessExt(name: string): string {
  for (const [re, ext] of EXT_HINTS) if (re.test(name)) return ext;
  return "bin";
}

export const importService = {
  /** 通过系统文件选择对话框选取文件（Tauri 内） */
  async pickFile(): Promise<string[] | null> {
    if (!isTauri()) return null;
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({
        multiple: true,
        filters: [
          {
            name: "eBooks",
            extensions: [
              "epub", "pdf", "mobi", "azw", "azw3", "txt", "fb2",
              "cbr", "cbz", "zip", "docx", "doc", "pptx", "ppt",
              "xlsx", "xls", "rtf", "odt", "ods", "odp", "md",
              "html", "htm", "xml", "mhtml", "mht",
            ],
          },
        ],
      });
      if (Array.isArray(selected)) return selected.length ? selected : null;
      return typeof selected === "string" ? [selected] : null;
    } catch {
      return null;
    }
  },

  /**
   * 启动导入（传入文件路径或 Android content:// URI）。
   *
   * Android SAF：content:// 直交 Rust `import_book_from_uri`（流式拷贝 + 零 IPC 字节），
   * 失败时兜底走「读字节 → 临时文件 → start_import_book」。
   * 桌面/iOS：file:// 规范化后走 start_import_book（原地引用，最快）。
   */
  async startImport(path: string, displayName?: string | null): Promise<ImportTask> {
    const raw = path;
    const resolved = normalizePath(raw);
    let fileName = displayName || fallbackName(raw);
    if (isTauri()) {
      try {
        const realName = await resolveContentUriName(raw);
        if (realName) fileName = realName;
      } catch (e) {
      logError("importService.realName", e);
  }
    }

    const task: ImportTask = {
      id: `imp-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
      fileName,
      progress: 0,
      speedKbps: 0,
      remainingSec: 0,
      status: "importing",
    };

    try {
      if (isTauri()) {
        if (/^content:\/\//i.test(resolved)) {
          try {
            await invoke<string>(CMD.importBookFromUri, {
              uri: resolved,
              displayName: fileName || null,
            });
          } catch (e) {
            logError("importService.startImport.uriFallback", e);
            // ROM 的 SAF getFileDescriptor 不可用 → 临时文件通道
            await fallbackImportViaTempFile(resolved, fileName || null);
          }
        } else {
          await invoke<void>(CMD.startImportBook, {
            filePath: resolved,
            displayName: fileName || null,
          });
        }
      }
    } catch (e) {
      task.status = "error";
      throw e;
    }
    return task;
  },

  /**
   * 监听导入事件（import-progress / import-done / import-error / import-skipped），
   * 按 fileName 匹配任务并回调。返回取消函数。
   */
  async listenImportEvents(onEvent: (e: ImportStatusEvent) => void): Promise<() => void> {
    if (!isTauri()) return () => {};
    const { listen } = await import("@tauri-apps/api/event");
    const unlisteners: Array<() => void> = [];
    for (const name of ["import-progress", "import-done", "import-error", "import-skipped"]) {
      try {
        const un = await listen<ImportStatusEvent>(name, (ev) => onEvent(ev.payload));
        unlisteners.push(un);
      } catch (e) {
        logError(`importService.listenImportEvents.${name}`, e);
      }
    }
    return () => unlisteners.forEach((un) => un());
  },

  async startLanServer(): Promise<string | null> {
    if (!isTauri()) return null;
    try {
      await invoke<void>(CMD.lanFileServerStart, {});
      return await invoke<string>(CMD.lanFileServerGetUrl, {});
    } catch (e) {
      // 向上抛，让 UI 显示失败原因
      throw e;
    }
  },

  /** 停止局域网文件服务器（关闭开关时调用） */
  async stopLanServer(): Promise<void> {
    if (!isTauri()) return;
    try {
      await invoke<void>(CMD.lanFileServerStop, {});
    } catch (e) {
  logError("importService.un", e);
  }
  },
};

/** 供 ImportPage 使用：把后端 stage 映射为前端 task 状态 */
export function mapImportStage(stage: string): ImportTask["status"] {
  switch (stage) {
    case "Done":
      return "done";
    case "Skipped":
      return "skipped";
    case "Failed":
    case "Cancelled":
      return "error";
    default:
      return "importing";
  }
}

/** 供 ImportPage 使用：根据文件名猜扩展名（临时文件兜底场景） */
export { guessExt };