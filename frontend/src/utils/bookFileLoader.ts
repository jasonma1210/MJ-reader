import { invoke } from "../services/tauri";
import { logError } from "./logError";

/**
 * 统一文件读取层（移植自 frontend-deprecated）。
 *
 * Tauri 2.x 的 read_file_bytes 在不同平台/WebView 上返回类型不一致：
 * - 桌面端 Chromium WebView：通常为 ArrayBuffer
 * - Android WebView（content:// URI）：可能为 number[] 或 Uint8Array
 * 本工具统一处理这些差异，对外只暴露 Uint8Array / ArrayBuffer / 文本三种形态。
 */
export interface BookFileData {
  /** 原始字节，统一为 Uint8Array */
  bytes: Uint8Array;
  /** ArrayBuffer 视图（与 bytes 共享内存，传给 xlsx/pdfjs 等库） */
  arrayBuffer: ArrayBuffer;
  /** 文本格式直接解码（txt/md/html/htm）；二进制格式为 null */
  text: string | null;
  /** 文件格式（小写） */
  format: string;
}

const TEXT_FORMATS = new Set(["txt", "md", "markdown", "html", "htm"]);

/** 将 invoke 返回的多种形态统一转为 Uint8Array */
function toUint8Array(input: unknown): Uint8Array {
  if (input === null || input === undefined) {
    return new Uint8Array(0);
  }
  if (input instanceof Uint8Array) {
    return input;
  }
  if (input instanceof ArrayBuffer) {
    return new Uint8Array(input);
  }
  if (typeof ArrayBuffer !== "undefined" && ArrayBuffer.isView(input)) {
    const view = input as ArrayBufferView;
    if (view.buffer instanceof ArrayBuffer) {
      return new Uint8Array(view.buffer, view.byteOffset, view.byteLength);
    }
  }
  if (Array.isArray(input)) {
    return new Uint8Array(input as number[]);
  }
  if (
    typeof input === "object" &&
    typeof (input as { length?: unknown }).length === "number"
  ) {
    const obj = input as { length: number; [k: number]: number };
    const len = obj.length;
    const arr = new Uint8Array(len);
    for (let i = 0; i < len; i++) {
      arr[i] = obj[i] ?? 0;
    }
    return arr;
  }
  throw new Error(
    `bookFileLoader: 无法识别的 invoke 返回类型 (${Object.prototype.toString.call(input)})`,
  );
}

/** 读取图书文件，统一通过 Rust 后端 `read_file_bytes` 命令 */
export async function loadBookFile(
  filePath: string,
  format: string,
): Promise<BookFileData> {
  const fmt = format.toLowerCase();

  if (TEXT_FORMATS.has(fmt)) {
    const command = fmt === "md" || fmt === "markdown" ? "read_markdown" : "read_txt";
    try {
      const text = await invoke<string>(command, { filePath });
      const raw = await invoke<unknown>("read_file_bytes", { filePath });
      const bytes = toUint8Array(raw);
      return {
        bytes,
        arrayBuffer: bytes.buffer.slice(
          bytes.byteOffset,
          bytes.byteOffset + bytes.byteLength,
        ) as ArrayBuffer,
        text,
        format: fmt,
      };
    } catch (e) {
      logError("bookFileLoader.loadBookFile", e);
      const raw = await invoke<unknown>("read_file_bytes", { filePath });
      const bytes = toUint8Array(raw);
      const text = new TextDecoder("utf-8").decode(bytes);
      return {
        bytes,
        arrayBuffer: bytes.buffer.slice(
          bytes.byteOffset,
          bytes.byteOffset + bytes.byteLength,
        ) as ArrayBuffer,
        text,
        format: fmt,
      };
    }
  }

  const raw = await invoke<unknown>("read_file_bytes", { filePath });
  const bytes = toUint8Array(raw);
  const arrayBuffer = bytes.buffer.slice(
    bytes.byteOffset,
    bytes.byteOffset + bytes.byteLength,
  ) as ArrayBuffer;

  return {
    bytes: new Uint8Array(arrayBuffer),
    arrayBuffer,
    text: null,
    format: fmt,
  };
}

/** 仅读取字节（不做格式判断） */
export async function loadBookBytes(filePath: string): Promise<{
  bytes: Uint8Array;
  arrayBuffer: ArrayBuffer;
}> {
  const raw = await invoke<unknown>("read_file_bytes", { filePath });
  const bytes = toUint8Array(raw);
  const arrayBuffer = bytes.buffer.slice(
    bytes.byteOffset,
    bytes.byteOffset + bytes.byteLength,
  ) as ArrayBuffer;
  return { bytes: new Uint8Array(arrayBuffer), arrayBuffer };
}
