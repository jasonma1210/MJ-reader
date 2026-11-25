// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
  useMemoryGuard,
  MEMORY_RELEASE_EVENT,
  MEMORY_RESTORE_EVENT,
  MEMORY_WARNING_EVENT,
  type MemoryEventDetail,
} from "../useMemoryGuard";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

function Probe() {
  useMemoryGuard();
  return null;
}

let root: Root | null = null;
let container: HTMLDivElement | null = null;

function mount(): void {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  act(() => root?.render(<Probe />));
}

function setVisibility(value: "hidden" | "visible"): void {
  Object.defineProperty(document, "visibilityState", {
    configurable: true,
    value,
  });
  act(() => {
    document.dispatchEvent(new Event("visibilitychange"));
  });
}

function listen(): { seen: MemoryEventDetail[]; stop: () => void } {
  const seen: MemoryEventDetail[] = [];
  const collect = (e: Event) => seen.push((e as CustomEvent<MemoryEventDetail>).detail);
  window.addEventListener(MEMORY_RELEASE_EVENT, collect);
  window.addEventListener(MEMORY_RESTORE_EVENT, collect);
  return {
    seen,
    stop: () => {
      window.removeEventListener(MEMORY_RELEASE_EVENT, collect);
      window.removeEventListener(MEMORY_RESTORE_EVENT, collect);
    },
  };
}

describe("useMemoryGuard", () => {
  beforeEach(() => setVisibility("visible"));

  afterEach(() => {
    if (root) act(() => root?.unmount());
    root = null;
    container?.remove();
    container = null;
  });

  it("maps visibility changes and warning events to release/restore", () => {
    mount();
    const { seen, stop } = listen();
    setVisibility("hidden");
    act(() => {
      window.dispatchEvent(new Event(MEMORY_WARNING_EVENT));
    });
    setVisibility("visible");
    stop();

    expect(seen.map((d) => d.reason)).toEqual(["background", "warning", "foreground"]);
    expect(seen.every((d) => typeof d.at === "number")).toBe(true);
  });

  it("removes listeners on unmount", () => {
    mount();
    act(() => root?.unmount());
    root = null;
    const { seen, stop } = listen();
    act(() => {
      window.dispatchEvent(new Event(MEMORY_WARNING_EVENT));
    });
    stop();
    expect(seen).toHaveLength(0);
  });
});
