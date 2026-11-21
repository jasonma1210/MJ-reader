/**
 * 目录批量导入契约测试（2026-11-20）。
 *
 * 背景：导入页新增"从目录导入"选项——用户点选目录（非手填）→ 后端 import_folders
 * 递归扫描。Android 上 tauri-plugin-dialog 在 #[cfg(mobile)] 下明确抛
 * FolderPickerNotImplemented，必须转成确定信号让 UI 诚实降级，不得臆造成功。
 *
 * 契约：
 *  ① 桌面：dialog.open({directory:true}) 返回路径 → pickFolder 原样返回；
 *  ② 移动端：open 抛 FolderPickerNotImplemented → 转 FolderPickerUnavailableError；
 *  ③ 用户取消（返回 null）→ pickFolder 返回 null；
 *  ④ importFolders 用 CMD.importFolders 且把 paths 原样透传。
 */
import { describe, it, expect, vi, beforeEach } from "vitest";

const openMock = vi.fn();
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: (...args: unknown[]) => openMock(...args),
}));

const invokeMock = vi.fn();
vi.mock("../services/tauri", () => ({
  CMD: {
    importFolders: "import_folders",
    getContentUriMetadata: "get_content_uri_metadata",
    startImportBook: "start_import_book",
    importBookFromUri: "import_book_from_uri",
  },
  invoke: (...args: unknown[]) => invokeMock(...args),
  isTauri: vi.fn(() => true),
}));

import { isTauri } from "../services/tauri";
import {
  importService,
  FolderPickerUnavailableError,
} from "../services/importService";

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(isTauri).mockReturnValue(true);
});

describe("importService.pickFolder", () => {
  it("①桌面返回用户选中的目录路径", async () => {
    openMock.mockResolvedValue("/Users/me/books");
    await expect(importService.pickFolder()).resolves.toBe("/Users/me/books");
    expect(openMock).toHaveBeenCalledWith({ directory: true, multiple: false });
  });

  it("②移动端 FolderPickerNotImplemented → 确定降级信号", async () => {
    openMock.mockRejectedValue(new Error("folder picker not implemented"));
    await expect(importService.pickFolder()).rejects.toBeInstanceOf(
      FolderPickerUnavailableError,
    );
  });

  it("③用户取消选择 → null", async () => {
    openMock.mockResolvedValue(null);
    await expect(importService.pickFolder()).resolves.toBeNull();
  });

  it("④非 Tauri 环境 → 返回 null 且不打开对话框", async () => {
    vi.mocked(isTauri).mockReturnValue(false);
    await expect(importService.pickFolder()).resolves.toBeNull();
    expect(openMock).not.toHaveBeenCalled();
  });
});

describe("importService.importFolders", () => {
  it("用 CMD.importFolders 且原样透传 paths，返回 ScanResult", async () => {
    const result = {
      totalFound: 12,
      imported: 9,
      skipped: 2,
      failed: 1,
      errors: ["bad.epub: x"],
    };
    invokeMock.mockResolvedValue(result);
    await expect(importService.importFolders(["/a", "/b"])).resolves.toEqual(result);
    expect(invokeMock).toHaveBeenCalledWith("import_folders", {
      paths: ["/a", "/b"],
    });
  });
});
