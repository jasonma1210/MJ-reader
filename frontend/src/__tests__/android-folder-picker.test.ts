// @vitest-environment jsdom
/**
 * Android SAF 目录桥前端契约测试（2026-11-20，JS 桥修订版）。
 *
 * 触发路径已绕开 Rust JNI（Tauri 2 下 ndk_context 未初始化，真机不可达）：
 * Kotlin 在 onWebViewCreate 中 addJavascriptInterface 注入 window.MjnFolderBridge。
 *
 * 契约：
 *  ① 桥未注入 → supported=false（UI 走桌面分流并诚实降级）；
 *  ② 桥注入且 ready → supported=true / ready=true；
 *  ③ 桥就绪 → 调 MjnFolderBridge.pick()，回调收集 URI 后逐条 startImport；
 *  ④ 用户取消（空数组）→ imported=0；
 *  ⑤ 单条导入失败 → 记入 failed/errors，不中断其余；
 *  ⑥ 桥未就绪 / 未触发 → 拒绝并清理悬挂回调。
 */
import { describe, it, expect, vi, beforeEach } from "vitest";

const invokeMock = vi.fn();
vi.mock("../services/tauri", () => ({
  CMD: {
    getContentUriMetadata: "get_content_uri_metadata",
    startImportBook: "start_import_book",
    importBookFromUri: "import_book_from_uri",
    importFolders: "import_folders",
  },
  invoke: (...args: unknown[]) => invokeMock(...args),
  isTauri: vi.fn(() => true),
}));

vi.mock("@tauri-apps/plugin-fs", () => ({
  stat: vi.fn(() => Promise.resolve({ name: "book.epub" })),
}));

import { importService } from "../services/importService";

type CallbackHost = Window & {
  __mjnFolderPicked?: (uris: string[]) => void;
  MjnFolderBridge?: { pick(): boolean; isReady(): boolean };
};

function setBridge(ready: boolean, pick?: () => boolean) {
  (window as CallbackHost).MjnFolderBridge = {
    isReady: vi.fn(() => ready),
    pick: vi.fn(pick ?? (() => true)),
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  delete (window as CallbackHost).__mjnFolderPicked;
  delete (window as CallbackHost).MjnFolderBridge;
  invokeMock.mockResolvedValue({
    displayName: "book.epub",
    size: 1024,
    mimeType: "application/epub+zip",
  });
});

describe("importService.androidFolderPickerStatus", () => {
  it("①桥未注入 → supported=false，不调用任何 Rust 命令", async () => {
    const status = await importService.androidFolderPickerStatus();
    expect(status).toEqual({
      triggered: false,
      ready: false,
      supported: false,
      reason: "Android folder bridge not injected",
    });
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("②桥注入且 ready → supported=true / ready=true", async () => {
    setBridge(true);
    const status = await importService.androidFolderPickerStatus();
    expect(status).toEqual({
      triggered: false,
      ready: true,
      supported: true,
      reason: null,
    });
  });

  it("桥注入但未就绪 → supported=true / ready=false 并给出原因", async () => {
    setBridge(false);
    const status = await importService.androidFolderPickerStatus();
    expect(status.supported).toBe(true);
    expect(status.ready).toBe(false);
    expect(status.reason).toContain("not ready");
  });
});

describe("importService.importFromAndroidFolder", () => {
  it("③桥就绪 → 调 pick() 后回调收集 URI，逐条导入", async () => {
    setBridge(true, () => {
      // pick() 同步触发后立即模拟 Kotlin evaluateJavascript 回传。
      queueMicrotask(() => {
        (window as CallbackHost).__mjnFolderPicked?.([
          "content://tree/a/document/1",
          "content://tree/a/document/2",
        ]);
      });
      return true;
    });

    const startSpy = vi.spyOn(importService, "startImport");
    const result = await importService.importFromAndroidFolder();
    expect(result.totalFound).toBe(2);
    expect(result.imported).toBe(2);
    expect((window as CallbackHost).MjnFolderBridge?.pick).toHaveBeenCalledTimes(1);
    expect(startSpy).toHaveBeenCalledTimes(2);
    expect(startSpy).toHaveBeenCalledWith("content://tree/a/document/1");
    startSpy.mockRestore();
    // 回调已清理，避免悬挂。
    expect((window as CallbackHost).__mjnFolderPicked).toBeUndefined();
  });

  it("④用户取消（空数组）→ imported=0", async () => {
    setBridge(true, () => {
      queueMicrotask(() => (window as CallbackHost).__mjnFolderPicked?.([]));
      return true;
    });
    const result = await importService.importFromAndroidFolder();
    expect(result.totalFound).toBe(0);
    expect(result.imported).toBe(0);
  });

  it("⑤单条失败记入 failed/errors，不中断其余", async () => {
    setBridge(true, () => {
      queueMicrotask(() =>
        (window as CallbackHost).__mjnFolderPicked?.(["content://a/1", "content://a/2"]),
      );
      return true;
    });
    const startSpy = vi
      .spyOn(importService, "startImport")
      .mockResolvedValueOnce({ id: "x", fileName: "x", progress: 0, speedKbps: 0, remainingSec: 0, status: "importing" })
      .mockRejectedValueOnce(new Error("read failed"));
    const result = await importService.importFromAndroidFolder();
    expect(result.imported).toBe(1);
    expect(result.failed).toBe(1);
    expect(result.errors[0]).toContain("read failed");
    startSpy.mockRestore();
  });

  it("⑥桥未注入 → 拒绝并清理回调", async () => {
    await expect(importService.importFromAndroidFolder()).rejects.toThrow("未注入");
    expect((window as CallbackHost).__mjnFolderPicked).toBeUndefined();
  });

  it("桥未就绪 → 拒绝且不调用 pick()", async () => {
    setBridge(false);
    await expect(importService.importFromAndroidFolder()).rejects.toThrow("未就绪");
    expect((window as CallbackHost).MjnFolderBridge?.pick).not.toHaveBeenCalled();
  });

  it("pick() 返回 false → 拒绝并清理回调", async () => {
    setBridge(true, () => false);
    await expect(importService.importFromAndroidFolder()).rejects.toThrow("未触发");
    expect((window as CallbackHost).__mjnFolderPicked).toBeUndefined();
  });
});
