import { describe, expect, it } from "vitest";
import { selectFragmentPreset } from "../src/fragment-selection.js";

describe("explicit fragment preset selection", () => {
  const options = { name: "article", collections: ["posts"], fragmentFieldsByCollection: { posts: { intro: "fragment-readable", body: "article" } } };
  it("selects each configured field independently", () => {
    expect(selectFragmentPreset(options, "posts", "intro")).toBe("fragment-readable");
    expect(selectFragmentPreset(options, "posts", "body")).toBe("article");
  });
  it("never falls back or selects inherited properties", () => {
    expect(selectFragmentPreset(options, "posts", "unknown")).toBeUndefined();
    expect(selectFragmentPreset(options, "posts", "toString")).toBeUndefined();
    expect(selectFragmentPreset(options, "toString", "intro")).toBeUndefined();
    expect(selectFragmentPreset(undefined, "posts", "intro")).toBeUndefined();
    expect(selectFragmentPreset({ ...options, collections: [] }, "posts", "intro")).toBeUndefined();
    expect(selectFragmentPreset({ name: "article", fragmentFieldsByCollection: { posts: { intro: " " } } }, "posts", "intro")).toBeUndefined();
  });
});
