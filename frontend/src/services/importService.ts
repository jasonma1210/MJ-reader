import { CMD, invoke, isTauri } from "./tauri";
import type { ImportTask } from "../types";
import { logError } from "../utils/logError";

/** Android content:// URI 元数据（Tauri 序列化为 camelCase：displayName / size / mimeType） */
export interface ContentUriMeta {
  displayName: string;
  size: number;
  mimeType: string;
}

/**
 * 目录批量扫描结果（对齐后端 directory::ScanResult，camelCase）。
 * 后端 import_folders 直接返回该结构，不走 import-progress 事件流。
 */
export interface FolderScanResult {
  totalFound: number;
  imported: number;
  skipped: number;
  failed: number;
  errors: string[];
}

/** 已绑定的书库目录（后端 library_dirs，camelCase） */
export interface LibraryDir {
  id: string;
  path: string;
  label: string | null;
  autoScan: boolean;
  createdAt: number;
}

/** 移动端不支持目录选择的确定信号（tauri-plugin-dialog 在 #[cfg(mobile)] 下抛此错误）。 */
export class FolderPickerUnavailableError extends Error {
  constructor() {
    super("folder picker unavailable on this platform");
    this.name = "FolderPickerUnavailableError";
  }
}

/**
 * Android 目录选择桥状态。
 * 触发路径已从 Rust JNI 命令改为 Kotlin addJavascriptInterface（见 MjnFolderBridge）。
 * triggered 字段为保持 UI 契约兼容恒为 false（UI 只读 supported/ready）。
 */
interface AndroidFolderPickerStatus {
  triggered: boolean;
  ready: boolean;
  supported: boolean;
  reason: string | null;
}

/** Kotlin 桥注入的全局回调名（FolderPickerBridge.kt 经 evaluateJavascript 调用）。 */
const ANDROID_FOLDER_CALLBACK = "__mjnFolderPicked" as const;

/**
 * Kotlin 经 addJavascriptInterface 暴露的桥对象（绕开 Rust JNI/JavaVM——Tauri 2 下不可用）。
 * 由 MainActivity 的 onWebViewCreate 在页面加载前注入 window.MjnFolderBridge。
 */
interface MjnFolderBridge {
  pick(): boolean;
  isReady(): boolean;
}

type FolderCallbackHost = Window & {
  [ANDROID_FOLDER_CALLBACK]?: (uris: string[]) => void;
  MjnFolderBridge?: MjnFolderBridge;
};

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
   * 探测 Android 原生目录选择桥是否可用。
   *
   * 触发路径为 Kotlin 在 WryActivity.onWebViewCreate 中经 addJavascriptInterface
   * 注入的 window.MjnFolderBridge（页面加载前完成）。Rust JNI 路径已废弃：
   * 真机 logcat 证实 Tauri 2 下 ndk_context 未初始化，JavaVM 不可获取。
   *
   * 非 Android / 桥未注入时返回 supported=false，UI 走桌面 plugin-dialog 分流。
   */
  async androidFolderPickerStatus(): Promise<AndroidFolderPickerStatus> {
    if (!isTauri()) {
      return { triggered: false, ready: false, supported: false, reason: null };
    }
    const bridge = (window as FolderCallbackHost).MjnFolderBridge;
    if (!bridge) {
      return {
        triggered: false,
        ready: false,
        supported: false,
        reason: "Android folder bridge not injected",
      };
    }
    let ready = false;
    try {
      ready = bridge.isReady();
    } catch (e) {
      logError("importService.androidFolderPickerStatus", e);
    }
    return {
      triggered: false,
      ready,
      supported: true,
      reason: ready ? null : "Android folder picker not ready",
    };
  },

  /**
   * 选择单个目录（导入页"从目录导入"选项，用户点选、不手填）。
   * - 桌面：plugin-dialog 的 open({directory:true})，返回真实路径。
   * - Android：plugin-dialog 在 #[cfg(mobile)] 下确定抛 FolderPickerNotImplemented；
   *   这里转成 FolderPickerUnavailableError，由 UI 走原生桥路径。
   */
  async pickFolder(): Promise<string | null> {
    if (!isTauri()) return null;
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({ directory: true, multiple: false });
      return typeof selected === "string" ? selected : null;
    } catch (e) {
      if (e instanceof Error && /folder.?picker.?not.?implemented/i.test(e.message)) {
        throw new FolderPickerUnavailableError();
      }
      throw e;
    }
  },

  /**
   * 递归扫描一个或多个目录并批量导入（桌面后端 import_folders）。
   * 返回聚合 ScanResult；不走 import-progress 事件流。
   */
  async importFolders(paths: string[]): Promise<FolderScanResult> {
    return invoke<FolderScanResult>(CMD.importFolders, { paths });
  },

  // ===== 目录绑定（2026-11-26）：只记录目录位置、原地读取，不复制书籍 =====

  async listLibraryDirs(): Promise<LibraryDir[]> {
    if (!isTauri()) return [];
    return invoke<LibraryDir[]>(CMD.listLibraryDirs);
  },

  /** 绑定目录：只 INSERT library_dirs，不导入任何书；随后由 scanLibraryDir 原地读取。 */
  async addLibraryDir(path: string, label?: string | null): Promise<LibraryDir> {
    return invoke<LibraryDir>(CMD.addLibraryDir, { path, label: label ?? null });
  },

  async removeLibraryDir(id: string): Promise<void> {
    return invoke<void>(CMD.removeLibraryDir, { id });
  },

  /** 扫描已绑定目录：递归原地引用图书（不复制），按 file_hash 去重。 */
  async scanLibraryDir(id: string): Promise<FolderScanResult> {
    return invoke<FolderScanResult>(CMD.scanLibraryDir, { id });
  },

  /** 启动同步所有绑定目录：原地读取新增书，且不复活用户已软删的书。 */
  async syncLibraryDirs(): Promise<FolderScanResult> {
    return invoke<FolderScanResult>(CMD.syncLibraryDirs);
  },

  /**
   * 从 Android 目录批量导入：触发 Kotlin JS 桥选择系统目录 → 等待桥回传 document URI 列表
   * → 逐条走现成的 startImport（content:// → import_book_from_uri，零字节过 IPC）。
   *
   * 回调由 FolderPickerBridge.kt 经 WebView.evaluateJavascript 注入，故此处先注册
   * window.__mjnFolderPicked，再同步调用 window.MjnFolderBridge.pick()。
   * 该路径完全绕开 Rust JNI —— 真机证实 Tauri 2 下 ndk_context 未初始化。
   */
  async importFromAndroidFolder(): Promise<FolderScanResult> {
    const host = window as FolderCallbackHost;
    const bridge = host.MjnFolderBridge;
    if (!bridge) {
      throw new Error("Android 目录选择桥未注入");
    }
    let ready = false;
    try {
      ready = bridge.isReady();
    } catch (e) {
      logError("importService.importFromAndroidFolder.isReady", e);
    }
    if (!ready) {
      throw new Error("Android 目录选择桥未就绪");
    }

    const uris = await new Promise<string[]>((resolve, reject) => {
      // 防止用户取消/超时后回调悬挂：60s 超时兜底。
      const timer = window.setTimeout(() => {
        delete host[ANDROID_FOLDER_CALLBACK];
        reject(new Error("Android 目录选择超时"));
      }, 60000);
      host[ANDROID_FOLDER_CALLBACK] = (result: string[]) => {
        window.clearTimeout(timer);
        delete host[ANDROID_FOLDER_CALLBACK];
        resolve(Array.isArray(result) ? result : []);
      };
      let triggered = false;
      try {
        triggered = bridge.pick();
      } catch (e) {
        window.clearTimeout(timer);
        delete host[ANDROID_FOLDER_CALLBACK];
        reject(e);
        return;
      }
      if (!triggered) {
        window.clearTimeout(timer);
        delete host[ANDROID_FOLDER_CALLBACK];
        reject(new Error("Android 目录选择未触发"));
      }
    });

    const result: FolderScanResult = {
      totalFound: uris.length,
      imported: 0,
      skipped: 0,
      failed: 0,
      errors: [],
    };
    for (const uri of uris) {
      try {
        await this.startImport(uri);
        result.imported += 1;
      } catch (e) {
        result.failed += 1;
        result.errors.push(`${uri}: ${e instanceof Error ? e.message : String(e)}`);
      }
    }
    return result;
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