/// <reference types="vite/client" />
import { describe, expect, it } from "vitest";

import { normalizeInteraction } from "../src/findings-view.js";

/**
 * Regression suite for the 0.8.17 production hotfix (commit 8825bd4,
 * "Harden handleAdminRoute against incomplete ctx.input").
 *
 * EmDash 0.17 reached /_emdash/api/plugins/aexeo-emdash/admin with
 * undefined, empty, and partial bodies on the admin and widget
 * hydration paths. The route read `body.type` straight off that value
 * and crashed with
 *
 *   TypeError: Cannot read properties of undefined (reading 'type')
 *
 * `normalizeInteraction` is the fix, and the CHANGELOG for 0.8.17
 * claims it was "verified against 10 edge-case inputs (undefined, null,
 * empty object, missing/empty page, missing action_id, missing values,
 * unknown type, string input, valid payload)" — but nothing covered
 * them. The expectations below are derived from the implementation in
 * src/findings-view.ts, not from the changelog prose.
 */
describe("normalizeInteraction", () => {
  it("falls back to the canonical landing for undefined", () => {
    expect(normalizeInteraction(undefined)).toEqual({
      type: "page_load",
      page: "/findings",
    });
  });

  it("falls back to the canonical landing for null", () => {
    expect(normalizeInteraction(null)).toEqual({
      type: "page_load",
      page: "/findings",
    });
  });

  it("falls back to the canonical landing for an empty object", () => {
    expect(normalizeInteraction({})).toEqual({
      type: "page_load",
      page: "/findings",
    });
  });

  it("substitutes /findings for a page_load with a missing page", () => {
    expect(normalizeInteraction({ type: "page_load" })).toEqual({
      type: "page_load",
      page: "/findings",
    });
  });

  it("substitutes /findings for a page_load with an empty page", () => {
    expect(normalizeInteraction({ type: "page_load", page: "" })).toEqual({
      type: "page_load",
      page: "/findings",
    });
  });

  it("substitutes an empty action_id for a block_action that omits it", () => {
    // The documented contract: `notFound("")` downstream, not a throw.
    expect(normalizeInteraction({ type: "block_action" })).toEqual({
      type: "block_action",
      action_id: "",
      value: undefined,
    });
  });

  it("substitutes an empty values object for a form_submit that omits it", () => {
    expect(normalizeInteraction({ type: "form_submit" })).toEqual({
      type: "form_submit",
      action_id: "",
      values: {},
    });
  });

  it("falls back to the canonical landing for an unknown type", () => {
    expect(
      normalizeInteraction({ type: "teleport", page: "/document" }),
    ).toEqual({ type: "page_load", page: "/findings" });
  });

  it("falls back to the canonical landing for a non-object input", () => {
    // A string body is truthy but not an object, so it must take the
    // same branch as null/undefined rather than reading `.type` off it.
    expect(normalizeInteraction("/findings")).toEqual({
      type: "page_load",
      page: "/findings",
    });
  });

  it("passes a valid payload through unchanged", () => {
    expect(
      normalizeInteraction({ type: "page_load", page: "/document" }),
    ).toEqual({ type: "page_load", page: "/document" });

    expect(
      normalizeInteraction({
        type: "block_action",
        action_id: "view_document",
        block_id: "b1",
        value: "/journal/hello/",
      }),
    ).toEqual({
      type: "block_action",
      action_id: "view_document",
      block_id: "b1",
      value: "/journal/hello/",
    });

    expect(
      normalizeInteraction({
        type: "form_submit",
        action_id: "view_document",
        values: { route_picker: "/journal/hello/" },
      }),
    ).toEqual({
      type: "form_submit",
      action_id: "view_document",
      values: { route_picker: "/journal/hello/" },
    });
  });

  it("omits block_id rather than stamping it as undefined", () => {
    // `exactOptionalPropertyTypes` makes `block_id: undefined` a
    // different type from an absent `block_id`; the implementation
    // uses a conditional spread so `in` checks and deep equality keep
    // working on the way out.
    const normalized = normalizeInteraction({
      type: "block_action",
      action_id: "refresh_findings",
    });
    expect(normalized).toEqual({
      type: "block_action",
      action_id: "refresh_findings",
      value: undefined,
    });
    expect(Object.hasOwn(normalized, "block_id")).toBe(false);
  });

  it("never throws, whatever it is handed", () => {
    const inputs: unknown[] = [
      undefined,
      null,
      {},
      "",
      "not-an-interaction",
      0,
      1,
      true,
      false,
      [],
      ["page_load"],
      { type: null },
      { type: 42 },
      { type: "page_load", page: 42 },
      { type: "page_load", page: null },
      { type: "block_action", action_id: 7 },
      { type: "block_action", block_id: 7 },
      { type: "form_submit", values: "nope" },
      { type: "form_submit", values: null },
    ];
    for (const input of inputs) {
      expect(() => normalizeInteraction(input)).not.toThrow();
      expect(normalizeInteraction(input)).toHaveProperty("type");
    }
  });
});

/**
 * The 0.8.17 hotfix shipped as two copies of normalizeInteraction — one
 * per entrypoint — and nothing kept them in step. That is how the Block
 * Kit refresh sweep later lost its suppression filter while the React
 * /refresh route kept it. Guard the de-duplication directly so a
 * well-meaning re-copy is a failing test rather than a silent bug.
 *
 * The sources come in through vite's `?raw` import rather than
 * node:fs: the vitest worker here runs with a virtualized cwd, so a
 * plain readFile of the project tree resolves to a path that does not
 * exist inside the worker.
 */
const sources = import.meta.glob("../src/*.ts", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

const ENTRYPOINTS = ["../src/configured.ts", "../src/sandbox-entry.ts"];

describe("shared view layer", () => {
  it("defines normalizeInteraction in exactly one place", () => {
    const defined = Object.entries(sources).filter(([, source]) =>
      /function normalizeInteraction\s*\(/.test(source),
    );
    expect(defined.map(([path]) => path)).toEqual([
      "../src/findings-view.ts",
    ]);
  });

  it("has both entrypoints import the shared definition", () => {
    for (const path of ENTRYPOINTS) {
      const source = sources[path];
      expect(source, `missing source for ${path}`).toBeDefined();
      expect(source).toMatch(/from "\.\/findings-view\.js"/);
      expect(source).toMatch(/\bnormalizeInteraction\b/);
      expect(source).not.toMatch(/function normalizeInteraction\s*\(/);
    }
  });

  it("passes a suppression filter to the shared refresh sweep from both entries", () => {
    // The regression: configured.ts's private copy of handleRefresh
    // never forwarded the filter, so the Block Kit Refresh button wrote
    // unsuppressed findings to KV while the React /refresh route
    // honoured them. RefreshOptions.suppressionFilter is required in the
    // type, so the omission is now a compile error; this asserts the
    // value is actually threaded into the call, not just mentioned.
    for (const path of ENTRYPOINTS) {
      const source = sources[path] ?? "";
      const call = /\b(refreshSweep|handleRefresh)\(\{/.exec(source);
      expect(call, `${path} has no refresh sweep call`).not.toBeNull();
      const body = source.slice(call?.index ?? 0, (call?.index ?? 0) + 400);
      expect(body).toMatch(/suppressionFilter/);
    }
  });
});
