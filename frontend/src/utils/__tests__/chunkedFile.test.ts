// @vitest-environment jsdom
/**
 * ChunkedBlob 大文件分块读取测试（漫画 OOM 根治）。
 *
 * 契约：
 *  ① shouldStream 只在体积 ≥32MB 时为真，未知体积保守 false；
 *  ② slice(start,end) 的负索引/越界钳制与原生 Blob 一致；
 *  ③ arrayBuffer() 按 8MB 分块调用 read_file_chunk，offset/length 递增正确；
 *  ④ 拼装结果与整块字节完全一致；
 *  ⑤ 区间超过文件尾时以实际返回长度为准。
 */
import { describe, it, expect, vi, beforeEach } from "vitest";

const invokeMock = vi.fn();
vi.mock("../../services/tauri", () => ({
  CMD: { readFileChunk: "read_file_chunk" },
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { ChunkedBlob, shouldStream, STREAM_THRESHOLD } from "../chunkedFile";

const CHUNK = 8 * 1024 * 1024;

beforeEach(() => {
  vi.clearAllMocks();
});

describe("shouldStream", () => {
  it("①小文件/未知体积不分块，大文件分块", () => {
    expect(shouldStream(1024)).toBe(false);
    expect(shouldStream(STREAM_THRESHOLD - 1)).toBe(false);
    expect(shouldStream(STREAM_THRESHOLD)).toBe(true);
    expect(shouldStream(200 * 1024 * 1024)).toBe(true);
    expect(shouldStream(null)).toBe(false);
    expect(shouldStream(undefined)).toBe(false);
  });
});

describe("ChunkedBlob.slice", () => {
  it("②负索引与越界钳制与原生 Blob 一致", () => {
    const b = new ChunkedBlob("/books/a.cbz", 1000);
    expect(b.size).toBe(1000);
    expect(b.slice(100, 200).size).toBe(100);
    expect(b.slice(-100).size).toBe(100);
    expect(b.slice(0, -900).size).toBe(100);
    expect(b.slice(900, 2000).size).toBe(100);
    expect(b.slice(200, 100).size).toBe(0);
  });

  it("③arrayBuffer() 按 8MB 分块调 read_file_chunk", async () => {
    const size = CHUNK * 2 + 100;
    const b = new ChunkedBlob("/books/big.cbz", size, "application/vnd.comicbook+zip");
    // 每块按请求长度返回填充字节
    invokeMock.mockImplementation(async (_cmd, args: { offset: number; length: number }) => {
      return new Uint8Array(args.length).fill(7);
    });

    const buf = await b.slice(0, size).arrayBuffer();
    expect(buf.byteLength).toBe(size);
    expect(invokeMock).toHaveBeenCalledTimes(3);
    expect(invokeMock).toHaveBeenNthCalledWith(1, "read_file_chunk", {
      filePath: "/books/big.cbz",
      offset: 0,
      length: CHUNK,
    });
    expect(invokeMock).toHaveBeenNthCalledWith(2, "read_file_chunk", {
      filePath: "/books/big.cbz",
      offset: CHUNK,
      length: CHUNK,
    });
    expect(invokeMock).toHaveBeenNthCalledWith(3, "read_file_chunk", {
      filePath: "/books/big.cbz",
      offset: CHUNK * 2,
      length: 100,
    });
  });

  it("④从非零 offset 切片时，块偏移从绝对位置开始", async () => {
    const size = CHUNK + 500;
    const b = new ChunkedBlob("/books/x.epub", size);
    invokeMock.mockImplementation(async (_cmd, args: { offset: number; length: number }) => {
      return new Uint8Array(args.length).fill(1);
    });
    await b.slice(100, CHUNK + 200).arrayBuffer();
    expect(invokeMock).toHaveBeenNthCalledWith(1, "read_file_chunk", {
      filePath: "/books/x.epub",
      offset: 100,
      length: CHUNK,
    });
    expect(invokeMock).toHaveBeenNthCalledWith(2, "read_file_chunk", {
      filePath: "/books/x.epub",
      offset: CHUNK + 100,
      length: 100,
    });
  });

  it("⑤Rust 返回不足（到文件尾）时以实际长度为准", async () => {
    const b = new ChunkedBlob("/books/short.mobi", CHUNK * 2);
    invokeMock.mockImplementation(async (_cmd, args: { length: number }) => {
      return new Uint8Array(Math.min(args.length, 50)).fill(3);
    });
    const buf = await b.slice(0, CHUNK * 2).arrayBuffer();
    expect(buf.byteLength).toBe(50);
    expect(invokeMock).toHaveBeenCalledTimes(1);
  });

  it("空切片不调用 IPC", async () => {
    const b = new ChunkedBlob("/books/a.cbz", 1000);
    const buf = await b.slice(500, 500).arrayBuffer();
    expect(buf.byteLength).toBe(0);
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("继承 Blob：第三方库的 instanceof 检查可通过", () => {
    const b = new ChunkedBlob("/books/a.cbz", 1000);
    expect(b).toBeInstanceOf(Blob);
    expect(b.name).toBe("a.cbz");
    expect(b.type).toBe("application/octet-stream");
  });
});
