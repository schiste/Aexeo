import { describe, expect, it } from "vitest";
import { collectionListStructureAttested, fragmentListStructureAttested } from "../src/list-structure-attestation.js";

describe("list structure host attestations", () => {
  it("requires an explicit matching collection", () => {
    expect(collectionListStructureAttested(["posts"], "posts")).toBe(true);
    for (const value of [undefined, null, "posts", {}, ["pages"]]) {
      expect(collectionListStructureAttested(value, "posts")).toBe(false);
    }
  });

  it("requires an explicit own collection and matching fragment field", () => {
    expect(fragmentListStructureAttested({ posts: ["intro"] }, "posts", "intro")).toBe(true);
    expect(fragmentListStructureAttested({ posts: ["intro"] }, "posts", "body")).toBe(false);
    for (const value of [undefined, null, [], { posts: "intro" }, Object.create({ posts: ["intro"] })]) {
      expect(fragmentListStructureAttested(value, "posts", "intro")).toBe(false);
    }
    expect(fragmentListStructureAttested({ pages: ["intro"] }, "posts", "intro")).toBe(false);
  });
});
