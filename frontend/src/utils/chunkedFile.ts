import { CMD, invoke } from "../services/tauri";

/**
 * 大文件分块读取（漫画 OOM 根治，2026-11-26）。
 *
 * 背景：Android WebView 的 Java 堆 growth limit 仅 256 MB，Rust 一次性把
 * 241–324 MB 的漫画作为 IPC 响应体送回时，wry JNI 的 byte_array_from_slice
 * 会在 Java 堆一次性分配等大 byte[] → OOM 闪退。
 *
 * 方案：实现一个继承 Blob 的虚拟 Blob——`slice()` 按块调 Rust `read_file_chunk`，
 * 每次 IPC ≤ 8 MB，峰值内存与文件体积解耦。zip.js 的 BlobReader 与 foliate
 * 的 MOBI/EPUB 解析器都只调 `blob.slice().arrayBuffer()`，因此完全透明。
 *
 * 适用范围：大于 STREAM_THRESHOLD 的二进制书籍（CBZ/EPUB/MOBI...）。
 * 小文件仍走 read_file_bytes 整块读取，避免分块往返开销。
 */

/** 超过该体积走分块（32 MB）：小文件整块读取更快，大文件避免堆峰值 */
export const STREAM_THRESHOLD = 32 * 1024 * 1024;

/** 与 Rust READ_CHUNK_MAX 对齐（8 MB） */
const CHUNK_SIZE = 8 * 1024 * 1024;

/**
 * 虚拟 Blob：按区间从 Rust 分块拉取，不持有整本字节。
 * 继承 Blob 仅为满足第三方库（foliate/zip.js）的 `instanceof Blob` 与类型契约；
 * 构造用 1 字节占位，真正的数据访问全部走覆写的 slice()。
 */
export class ChunkedBlob extends Blob {
  readonly fileSize: number;
  /** 供 foliate 的 basename() 判断格式（原生 Blob 无 name，这里补上） */
  readonly name: string;
  private readonly filePath: string;

  constructor(filePath: string, size: number, mime = "application/octet-stream") {
    // 占位 1 字节：原生 Blob 必须持有真实字节，但体积可忽略。
    super([new Uint8Array(1)], { type: mime });
    if (!Number.isFinite(size) || size < 0) {
      throw new Error(`ChunkedBlob: 非法 size ${size}`);
    }
    this.filePath = filePath;
    this.fileSize = size;
    const cleaned = filePath.split(/[?#]/)[0] ?? filePath;
    this.name = cleaned.split(/[/\\]/).pop() || "book";
  }

  /** HTTP 语义的 size：第三方库据此判断 zip EOCD 位置、记录数等 */
  override get size(): number {
    return this.fileSize;
  }

  /**
   * 与 Blob.slice() 一致：支持负索引、越界钳制、start>end 返回空。
   * 返回对象只需满足消费者实际用到的 arrayBuffer()/size/text()。
   */
  override slice(start?: number, end?: number): Blob {
    const size = this.fileSize;
    let s = start ?? 0;
    let e = end ?? size;
    if (s < 0) s = Math.max(size + s, 0);
    if (e < 0) e = Math.max(size + e, 0);
    s = Math.min(Math.max(s, 0), size);
    e = Math.min(Math.max(e, 0), size);
    if (e < s) e = s;
    const length = e - s;
    const path = this.filePath;
    const type = this.type;

    // 构造一个只实现 Blob 读取接口的轻量对象；zip.js/foliate 不会检查
    // instanceof Blob，只调 slice(...).arrayBuffer()。
    const view = {
      size: length,
      type,
      arrayBuffer: async (): Promise<ArrayBuffer> => {
        if (length === 0) return new ArrayBuffer(0);
        const out = new Uint8Array(length);
        let filled = 0;
        while (filled < length) {
          const want = Math.min(CHUNK_SIZE, length - filled);
          const part = await invoke<unknown>(CMD.readFileChunk, {
            filePath: path,
            offset: s + filled,
            length: want,
          });
          const bytes = toBytes(part);
          if (bytes.length === 0) break; // 到达文件尾
          const n = Math.min(bytes.length, length - filled);
          out.set(bytes.subarray(0, n), filled);
          filled += n;
          if (bytes.length < want) break; // Rust 返回不足 = 文件尾
        }
        return out.buffer.slice(0, filled) as ArrayBuffer;
      },
      text: async (): Promise<string> => {
        const buf = await view.arrayBuffer();
        return new TextDecoder("utf-8").decode(buf);
      },
      slice: (s2?: number, e2?: number): Blob => {
        // 嵌套 slice：基于绝对区间再切一次
        const a = s + (s2 ?? 0);
        const b = s + (e2 ?? length);
        return this.slice(a, b);
      },
      stream: (): ReadableStream<Uint8Array> => {
        throw new Error("ChunkedBlob: stream() 不支持，请使用 arrayBuffer()");
      },
      bytes: async (): Promise<Uint8Array> => new Uint8Array(await view.arrayBuffer()),
      arrayBufferView: (): Uint8Array => {
        throw new Error("ChunkedBlob: arrayBufferView() 不支持");
      },
    };
    return view as unknown as Blob;
  }
}

/** 将 IPC 返回的多种形态统一为 Uint8Array（Android WebView 可能返回 number[]） */
function toBytes(input: unknown): Uint8Array {
  if (input == null) return new Uint8Array(0);
  if (input instanceof Uint8Array) return input;
  if (input instanceof ArrayBuffer) return new Uint8Array(input);
  if (ArrayBuffer.isView(input)) {
    const v = input as ArrayBufferView;
    return new Uint8Array(v.buffer, v.byteOffset, v.byteLength);
  }
  if (Array.isArray(input)) return new Uint8Array(input as number[]);
  throw new Error("ChunkedBlob: 无法识别的分块返回类型");
}

/**
 * 分块读取整本为 JS Uint8Array（PDF 等必须一次性拿到 bytes 的消费者）。
 *
 * 关键：每块 IPC ≤8 MB，Android WebView 的 Java 堆峰值与文件大小解耦；
 * 拼接结果留在 JS 堆（V8 堆上限远高于 Java heap growth limit 256 MB），
 * 再由 pdfjs 转移到 worker。漫画 PDF（图片型大文件）走此路径避免 OOM。
 */
export async function readAllBytesChunked(filePath: string, size: number): Promise<Uint8Array> {
  if (!Number.isFinite(size) || size <= 0) return new Uint8Array(0);
  const out = new Uint8Array(size);
  let filled = 0;
  while (filled < size) {
    const want = Math.min(CHUNK_SIZE, size - filled);
    const part = await invoke<unknown>(CMD.readFileChunk, {
      filePath,
      offset: filled,
      length: want,
    });
    const bytes = toBytes(part);
    if (bytes.length === 0) break;
    const n = Math.min(bytes.length, want);
    out.set(bytes.subarray(0, n), filled);
    filled += n;
    if (bytes.length < want) break; // Rust 返回不足 = 文件尾
  }
  return filled === size ? out : out.subarray(0, filled);
}

/**
 * 判断某文件是否应走分块流式：体积已知且 ≥ STREAM_THRESHOLD。
 * 体积未知时返回 false（保守走整块），避免误判。
 */
export function shouldStream(size: number | null | undefined): boolean {
  return (
    typeof size === "number" &&
    Number.isFinite(size) &&
    size >= STREAM_THRESHOLD
  );
}
