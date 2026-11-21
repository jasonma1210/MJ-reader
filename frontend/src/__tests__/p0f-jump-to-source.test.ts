/**
 * P0-F 一键回原文：useJumpToSource 单测（node 环境直接把 hook 当纯函数调用）。
 * ① 传 bookId+cfi → navigate 到 /reader/:bookId，并派发 {cfi} 定位事件
 * ② cfi 为空/空白 → 仍跳书（降级恢复进度），不派发定位事件
 * ③ cfi 首尾空白 → trim 后派发，detail 不含空 title
 * ④ bookId 为空 → 不跳转、不派发
 * ⑤ JSON percentage locator → 派发 {position} 而非把 JSON 当 CFI
 */
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

const { mockNavigate } = vi.hoisted(() => ({ mockNavigate: vi.fn() }));

// useCallback 恒等返回，使 hook 可在组件外直接调用；useNavigate 返回可控 mock
vi.mock("react", () => ({
  useCallback: (fn: (...args: unknown[]) => unknown) => fn,
}));
vi.mock("react-router-dom", () => ({
  useNavigate: () => mockNavigate,
}));

import { useJumpToSource } from "../hooks/useJumpToSource";

function makeFakeWindow() {
  return {
    setTimeout: vi.fn((_cb: () => void, _ms?: number) => {
      _cb(); // 立即执行定时器回调，便于断言派发
      return 1;
    }),
    dispatchEvent: vi.fn(),
  };
}
let fakeWindow: ReturnType<typeof makeFakeWindow>;

beforeEach(() => {
  vi.clearAllMocks();
  fakeWindow = makeFakeWindow();
  vi.stubGlobal("window", fakeWindow);
  vi.stubGlobal(
    "CustomEvent",
    class FakeCustomEvent {
      type: string;
      detail: unknown;
      constructor(type: string, init?: { detail?: unknown }) {
        this.type = type;
        this.detail = init?.detail;
      }
    },
  );
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("useJumpToSource（P0-F 统一回原文入口）", () => {
  it("① bookId+cfi：跳 /reader/:bookId 并派发 {cfi} 定位事件（400ms 延时）", () => {
    const jump = useJumpToSource();
    jump("book-1", "epubcfi(/6/4!/4/2:0)");

    expect(mockNavigate).toHaveBeenCalledTimes(1);
    expect(mockNavigate).toHaveBeenCalledWith("/reader/book-1");
    expect(fakeWindow.setTimeout).toHaveBeenCalledTimes(1);
    expect(fakeWindow.setTimeout.mock.calls[0][1]).toBe(400);
    expect(fakeWindow.dispatchEvent).toHaveBeenCalledTimes(1);
    const evt = fakeWindow.dispatchEvent.mock.calls[0][0] as {
      type: string;
      detail: unknown;
    };
    expect(evt.type).toBe("mjnexus:reader-scroll-to");
    expect(evt.detail).toEqual({ cfi: "epubcfi(/6/4!/4/2:0)" });
  });

  it("② cfi 为空/仅空白：仍跳书（降级恢复进度），不派发定位事件", () => {
    const jump = useJumpToSource();
    jump("book-2", "");
    jump("book-3", "   ");

    expect(mockNavigate).toHaveBeenCalledWith("/reader/book-2");
    expect(mockNavigate).toHaveBeenCalledWith("/reader/book-3");
    expect(fakeWindow.setTimeout).not.toHaveBeenCalled();
    expect(fakeWindow.dispatchEvent).not.toHaveBeenCalled();
  });

  it("③ cfi 首尾空白：trim 后派发，detail 不含空 title", () => {
    const jump = useJumpToSource();
    jump("book-4", "  pdf:12  ", "   ");

    const evt = fakeWindow.dispatchEvent.mock.calls[0][0] as {
      detail: unknown;
    };
    expect(evt.detail).toEqual({ cfi: "pdf:12" });
  });

  it("④ bookId 为空：不跳转、不派发", () => {
    const jump = useJumpToSource();
    jump("", "epubcfi(/6/4!/4/2:0)");

    expect(mockNavigate).not.toHaveBeenCalled();
    expect(fakeWindow.dispatchEvent).not.toHaveBeenCalled();
  });

  it("⑤ JSON percentage locator：派发 {position: percentage*100}，不伪造成 CFI", () => {
    const jump = useJumpToSource();
    jump("book-5", '{"percentage":0.42}');

    const evt = fakeWindow.dispatchEvent.mock.calls[0][0] as {
      detail: unknown;
    };
    expect(evt.detail).toEqual({ position: 42 });
  });
});
