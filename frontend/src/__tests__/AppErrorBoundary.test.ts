// @vitest-environment jsdom
import React from "react";
import { createRoot } from "react-dom/client";
import { act } from "react-dom/test-utils";
import { describe, expect, it, vi } from "vitest";
import i18n from "../i18n";
import { AppErrorBoundary } from "../components/common/AppErrorBoundary";

function Boom(): React.ReactElement {
  throw new Error("boom from child");
}

describe("AppErrorBoundary（P0-A 渲染崩溃兜底）", () => {
  it("子组件抛错时渲染 fallback，不再渲染子树，并记录 console.error", () => {
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    // React 自身也会 console.error；静音并断言我们的边界确实调用过
    // eslint-disable-next-line no-console
    const spy = vi.spyOn(console, "error").mockImplementation(() => {});
    try {
      act(() => {
        root.render(
          React.createElement(AppErrorBoundary, {
            source: "onboarding",
            children: React.createElement(
              "div",
              null,
              React.createElement(Boom),
              React.createElement("span", null, "should not render"),
            ),
          }),
        );
      });

      expect(container.querySelector('[role="alert"]')).not.toBeNull();
      expect(container.textContent).not.toContain("should not render");
      const labels = Array.from(container.querySelectorAll("button")).map(
        (b) => b.textContent,
      );
      expect(labels).toContain(i18n.t("errorBoundary.retry"));
      expect(labels).toContain(
        i18n.t("errorBoundary.retryClearOnboarding"),
      );
      expect(spy).toHaveBeenCalled();
    } finally {
      act(() => root.unmount());
      container.remove();
      spy.mockRestore();
    }
  });
});
