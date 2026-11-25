// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { Splash } from "../Splash";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

vi.mock("../../brand/MjDotConverge", () => ({
  MjDotConverge: () => <svg />,
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT =
  true;

describe("Splash skip", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
  });

  it("点击可跳过且不误触发动画完成回调", () => {
    const onSkip = vi.fn();
    const onAnimationDone = vi.fn();
    act(() => root.render(<Splash onSkip={onSkip} onAnimationDone={onAnimationDone} />));
    const overlay = container.firstElementChild as HTMLElement;
    expect(overlay.getAttribute("role")).toBe("button");
    act(() => overlay.click());
    expect(onSkip).toHaveBeenCalledTimes(1);
    expect(onAnimationDone).not.toHaveBeenCalled();
    act(() => root.unmount());
  });

  it("回车键可跳过", () => {
    const onSkip = vi.fn();
    act(() => root.render(<Splash onSkip={onSkip} />));
    const overlay = container.firstElementChild as HTMLElement;
    act(() => {
      overlay.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
      );
    });
    expect(onSkip).toHaveBeenCalledTimes(1);
    act(() => root.unmount());
  });

  it("未提供跳过回调时不暴露按钮语义", () => {
    act(() => root.render(<Splash />));
    const overlay = container.firstElementChild as HTMLElement;
    expect(overlay.getAttribute("role")).toBeNull();
    expect(overlay.textContent).not.toContain("splash.skipHint");
    act(() => root.unmount());
  });
});
