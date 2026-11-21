/**
 * S4-4.2 冒烟 · 链路 1：阅读器打开（mock 后端 get_book_by_id）。
 * 验证：ReaderPage 打开所需的 bookService.getBookById 链路在 Tauri 环境下
 * 命中后端命令、返回真实 Book，且命令参数键名与后端形参一致（id，非 bookId）。
 */
import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

import { invoke } from "@tauri-apps/api/core";
import { bookService } from "../services/bookService";
import { MOCK_BOOKS } from "../services/mock";

const mockedInvoke = vi.mocked(invoke);

beforeEach(() => {
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
  mockedInvoke.mockReset();
});

describe("smoke · reader open (mock backend)", () => {
  it("get_book_by_id 命中后端命令且返回书籍（ReaderPage 打开前提）", async () => {
    const book = { ...MOCK_BOOKS[0], id: "book-reader-smoke" };
    mockedInvoke.mockResolvedValue(book);

    const got = await bookService.getBookById("book-reader-smoke");

    expect(mockedInvoke).toHaveBeenCalledWith("get_book_by_id", {
      id: "book-reader-smoke",
    });
    expect(got?.title).toBe(book.title);
  });

  it("后端失败时降级为 null 而非抛错（ReaderPage 有错误态兜底）", async () => {
    mockedInvoke.mockRejectedValue(new Error("db locked"));
    const got = await bookService.getBookById("missing");
    expect(got).toBeNull();
  });
});
