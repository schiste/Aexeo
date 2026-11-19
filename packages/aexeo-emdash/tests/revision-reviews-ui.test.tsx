import { describe, expect, it } from "vitest";
import { RevisionReviewInstructions, ReviewHistory } from "../src/admin/RevisionReviews.js";

describe("revision review UI", () => {
  it("explains that reviews do not suppress findings or carry over", () => {
    const view = JSON.stringify(RevisionReviewInstructions());
    expect(view).toContain("Revision audit and editorial review");
    expect(view).toContain("never suppress them");
    expect(view).toContain("another audit");
  });
});


describe("review history presentation", () => {
  it("labels incomplete, attributed and unattributed submissions without implying publication approval", () => {
    const view = JSON.stringify(ReviewHistory({ events: [
      { event_id: "1", recorded_at: "2026-01-01", decision: "retain_with_explanation", explanation: "Intentional.", actor: { kind: "authenticated_account", id: "editor" } },
      { event_id: "2", recorded_at: "2026-01-02", decision: "needs_revision", explanation: "Revise.", actor: { kind: "unattributed" } },
    ] }));
    expect(view).toContain("History may be incomplete");
    expect(view).toContain("not publication approvals");
    expect(view).toContain("Account: editor");
    expect(view).toContain("Unattributed");
    expect(view).toContain("Intentional.");
  });
  it("distinguishes no recorded submissions from an unavailable history", () => {
    expect(JSON.stringify(ReviewHistory({ events: [] }))).toContain("No recorded submissions");
  });
});
