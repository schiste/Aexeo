import { describe, expect, it } from "vitest";

import { scoreSite } from "../src/evaluator-sandbox.js";
import { renderScoreWidget } from "../src/findings-view.js";
import { documentKey, type KvNamespace } from "../src/plugin.js";

describe("sandbox score stub", () => {
  it("makes the widget report unavailable instead of a score of 0", async () => {
    const entry = {
      key: documentKey("/a"),
      value: {
        document: { route: "/a", title: "A" },
        meta: { id: "a", collection: "pages", slug: "a", status: "published", title: "A", publicUrl: null },
      },
    };
    const kv = { list: async () => [entry] } as unknown as KvNamespace;

    const response = await renderScoreWidget(kv, {
      emptyFindingsText: "",
      emptyScoreText: "",
      refreshFailureVariant: "alert",
      score: (documents) => scoreSite(documents),
      truthLabel: () => "Truth",
    });

    const rendered = JSON.stringify(response.blocks);
    expect(rendered).toMatch(/unavailable/);
    expect(rendered).not.toMatch(/Site score is 0|"Overall"/);
  });
});
