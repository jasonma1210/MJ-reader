// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { capabilityFor, type TypographyItem } from "../TypographyPopover";

const ALL: TypographyItem[] = [
  "viewMode", "fontSize", "fontFamily", "lineHeight", "paraSpacing",
  "bgColor", "align", "letterSpacing", "verticalWriting",
];

describe("TypographyPopover capability matrix", () => {
  it("PDF only enables background and explains every disabled item", () => {
    const cap = capabilityFor("pdf");
    expect([...cap.enabled]).toEqual(["bgColor"]);
    for (const item of ALL.filter((x) => x !== "bgColor")) {
      expect(cap.enabled.has(item)).toBe(false);
      expect(cap.reasons[item]).toBeTruthy();
    }
  });

  it("Office enables real reflow typography but disables alignment and pagination", () => {
    const cap = capabilityFor("docx");
    for (const item of ["fontSize", "fontFamily", "lineHeight", "paraSpacing", "bgColor"] as TypographyItem[]) {
      expect(cap.enabled.has(item)).toBe(true);
    }
    for (const item of ["viewMode", "align", "letterSpacing", "verticalWriting"] as TypographyItem[]) {
      expect(cap.enabled.has(item)).toBe(false);
      expect(cap.reasons[item]).toBeTruthy();
    }
  });

  it("Text formats have the same matrix as Office", () => {
    expect([...capabilityFor("md").enabled].sort()).toEqual(
      [...capabilityFor("xlsx").enabled].sort(),
    );
  });

  it("EPUB keeps every typography item available", () => {
    const cap = capabilityFor("epub");
    expect(ALL.every((item) => cap.enabled.has(item))).toBe(true);
    expect(Object.keys(cap.reasons)).toHaveLength(0);
  });
});
