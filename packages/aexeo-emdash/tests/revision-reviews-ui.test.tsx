import { describe, expect, it } from "vitest";
import { RevisionReviewInstructions } from "../src/admin/RevisionReviews.js";

describe("revision review UI", () => {
  it("explains that reviews do not suppress findings or carry over", () => {
    const view = JSON.stringify(RevisionReviewInstructions());
    expect(view).toContain("Revision audit and editorial review");
    expect(view).toContain("never suppress them");
    expect(view).toContain("another audit");
  });
});
