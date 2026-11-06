import { describe, expect, it } from "vitest";

import {
  renderScoreWidget,
  type AdminViewTuning,
} from "../src/findings-view.js";
import type { KvNamespace } from "../src/plugin.js";
import type { SiteIntelligenceScore } from "../src/types.js";

const kv = {
  list: async () => [
    {
      key: "document:home",
      value: { route: "", title: "Home", blocks: [] },
    },
  ],
} as unknown as KvNamespace;

function tuningFor(score: SiteIntelligenceScore): AdminViewTuning {
  return {
    emptyScoreText: "empty",
    score: async () => score,
    truthLabel: () => "Truth",
  } as unknown as AdminViewTuning;
}

const lowScore: SiteIntelligenceScore = {
  citation_readiness_score: 10,
  truth_consistency_score: null,
  answer_pack: {
    routes_analyzed: 4,
    routes_with_answer_blocks: 1,
    routes_with_coverage_gaps: 3,
    answer_blocks: 2,
  },
  external_trust_alignment_score: null,
  route_scores: [],
  blockers: [{ message: "missing_direct_answer" }],
};

describe("renderScoreWidget", () => {
  it("reports facts and blockers with no composite or threshold alert", async () => {
    const { blocks } = await renderScoreWidget(kv, tuningFor(lowScore));

    expect(JSON.stringify(blocks)).not.toMatch(/overall|threshold/i);
    expect(blocks).not.toContainEqual(
      expect.objectContaining({ type: "banner" }),
    );
    expect(blocks).toEqual([
      {
        type: "stats",
        items: [
          { label: "Citation", value: "10" },
          { label: "Truth", value: "unknown" },
          { label: "Answers", value: "1/4 routes" },
        ],
      },
      { type: "context", text: "Top blockers: missing_direct_answer" },
    ]);
  });
});
