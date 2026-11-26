// @vitest-environment jsdom
import { act, type ReactElement, type RefObject } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { VisionPopover, type VisionPopoverProps } from "../VisionPopover";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

// React 18 显式声明 act 环境，避免 jsdom 下的 act 警告
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

/** 轻量 DOM 渲染器：项目未安装 @testing-library/dom，直接用 react-dom/client */
function render(ui: ReactElement) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  act(() => root.render(ui));
  return {
    container,
    unmount: () => {
      act(() => root.unmount());
      container.remove();
    },
  };
}

function buttonByText(container: HTMLElement, text: string): HTMLButtonElement {
  const found = Array.from(container.querySelectorAll("button")).find(
    (b) => b.textContent?.trim() === text,
  );
  if (!found) throw new Error(`button not found: ${text}`);
  return found;
}

function dialog(container: HTMLElement): HTMLElement {
  const found = container.querySelector('[role="dialog"]') as HTMLElement | null;
  if (!found) throw new Error("dialog not found");
  return found;
}

function makeProps(overrides: Partial<VisionPopoverProps> = {}): VisionPopoverProps {
  return {
    open: true,
    anchorRef: { current: null } as RefObject<HTMLElement | null>,
    onClose: vi.fn(),
    mode: "standard",
    onModeChange: vi.fn(),
    tempFollow: true,
    onTempFollowChange: vi.fn(),
    warmth: 0,
    onWarmthChange: vi.fn(),
    intensity: 60,
    onIntensityChange: vi.fn(),
    restMinutes: 0,
    onRestMinutesChange: vi.fn(),
    docKind: "epub",
    band: "day",
    ...overrides,
  };
}

function click(el: Element) {
  act(() => {
    el.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
}

describe("VisionPopover：面板开关与三分区渲染", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
  });

  it("open=false 时不渲染 dialog", () => {
    const { container } = render(<VisionPopover {...makeProps({ open: false })} />);
    expect(container.querySelector('[role="dialog"]')).toBeNull();
  });

  it("open=true 时渲染 dialog 与三个分区标题", () => {
    const { container } = render(<VisionPopover {...makeProps()} />);
    const panel = dialog(container);
    expect(panel.getAttribute("aria-label")).toBe("vision.panel.title");
    for (const key of [
      "vision.mode.title",
      "vision.temp.title",
      "vision.rhythm.title",
    ]) {
      expect(panel.textContent).toContain(key);
    }
  });

  it("视觉模式三态按钮存在，点击回写选择", () => {
    const props = makeProps();
    const { container } = render(<VisionPopover {...props} />);
    for (const key of ["standard", "simulated", "ink"]) {
      expect(buttonByText(container, `vision.mode.${key}`)).toBeTruthy();
    }
    click(buttonByText(container, "vision.mode.ink"));
    expect(props.onModeChange).toHaveBeenCalledWith("ink");
  });

  it("休息提醒分段可点击回写 30 分钟", () => {
    const props = makeProps();
    const { container } = render(<VisionPopover {...props} />);
    click(buttonByText(container, "vision.rhythm.m30"));
    expect(props.onRestMinutesChange).toHaveBeenCalledWith(30);
  });

  it("色温滑块与预览卡渲染；手动模式下 warmth 滑块启用", () => {
    const { container } = render(<VisionPopover {...makeProps({ tempFollow: false })} />);
    const panel = dialog(container);
    const warmth = panel.querySelector(
      '[role="slider"][aria-label="vision.temp.warmth"]',
    );
    const intensity = panel.querySelector(
      '[role="slider"][aria-label="vision.temp.intensity"]',
    );
    expect(warmth).toBeTruthy();
    expect(intensity).toBeTruthy();
    // 重复文案已移除：预览卡只显示一次简短标题，不再重复 mode.hint / 免责声明。
    expect(panel.textContent).toContain("vision.temp.preview");
    expect(panel.textContent?.match(/vision\.mode\.hint/g)).toHaveLength(1);
    expect(warmth?.getAttribute("aria-disabled")).toBe("false");
  });

  it("跟随时间模式下 warmth 滑块禁用", () => {
    const { container } = render(<VisionPopover {...makeProps({ tempFollow: true })} />);
    const warmth = dialog(container).querySelector(
      '[role="slider"][aria-label="vision.temp.warmth"]',
    );
    expect(warmth?.getAttribute("aria-disabled")).toBe("true");
  });

  it("Esc 与外部点击都能关闭面板", () => {
    const props = makeProps();
    render(<VisionPopover {...props} />);
    act(() => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    });
    expect(props.onClose).toHaveBeenCalledTimes(1);
    act(() => {
      document.body.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    });
    expect(props.onClose).toHaveBeenCalledTimes(2);
  });

  it("ink + EPUB 显示灰度高亮说明；PDF 隐藏", () => {
    const first = render(<VisionPopover {...makeProps({ mode: "ink", docKind: "epub" })} />);
    expect(dialog(first.container).textContent).toContain("vision.highlight.greyscale");
    first.unmount();

    const second = render(<VisionPopover {...makeProps({ mode: "ink", docKind: "pdf" })} />);
    expect(dialog(second.container).textContent).not.toContain("vision.highlight.greyscale");
  });

  it("预览卡应用合成滤镜（标准 + 日间自动色温）", () => {
    const { container } = render(
      <VisionPopover {...makeProps({ mode: "standard", band: "day", intensity: 100 })} />,
    );
    expect(dialog(container).querySelector('[style*="sepia"]')).toBeTruthy();
  });
});
