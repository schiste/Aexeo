import {
  createPluginRuntimeTestHost,
  type PluginRuntimeTestHost,
} from "@emdash-cms/plugin-test";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  adaptContentItemWithContext,
  type ContentUrlApi,
  type EmdashContentItem,
} from "../src/adapter.js";
import { buildAllowedHosts, buildCapabilities } from "../src/plugin.js";

let host: PluginRuntimeTestHost | undefined;

afterEach(async () => {
  await host?.dispose();
  host = undefined;
});

/**
 * Minimal single-paragraph Portable Text value. The adapter only recognizes
 * a body when it looks like a real rich-text array (`_type: "block"` with
 * typed spans), so the fixture must supply one — a bare `title` string is
 * deliberately not enough to populate `document.body`.
 */
function body(text: string): Array<Record<string, unknown>> {
  return [
    {
      _type: "block",
      _key: `block-${text}`,
      children: [{ _type: "span", _key: `span-${text}`, text, marks: [] }],
    },
  ];
}

describe("EmDash content adaptation", () => {
  it("declares only the current capabilities and configured sidecar host", () => {
    expect(buildCapabilities(null)).toEqual(["content:read"]);
    expect(buildAllowedHosts(null)).toEqual([]);
    expect(buildCapabilities("evaluator.test")).toEqual([
      "content:read",
      "network:request",
    ]);
    expect(buildAllowedHosts("evaluator.test")).toEqual(["evaluator.test"]);
  });

  it("keeps typed block text and resolves exact URLs and translation alternates", async () => {
    const urls = new Map([
      ["article-en", "https://www.aexeo.test/journal/hello/"],
      ["article-fr", "https://www.aexeo.test/fr/journal/bonjour/"],
    ]);
    const contentApi: ContentUrlApi = {
      getPublicUrl: vi.fn(async (_collection, id) => urls.get(id) ?? null),
      getTranslations: vi.fn(async () => ({
        translationGroup: "article-group",
        translations: [
          {
            id: "article-en",
            locale: "en",
            slug: "hello",
            status: "published",
            updatedAt: "2026-09-27T00:00:00.000Z",
          },
          {
            id: "article-fr",
            locale: "fr",
            slug: "bonjour",
            status: "published",
            updatedAt: "2026-09-27T00:00:00.000Z",
          },
        ],
      })),
    };
    const item: EmdashContentItem = {
      id: "article-en",
      type: "posts",
      slug: "hello",
      status: "published",
      locale: "en",
      data: {
        title: "Hello",
        blocks: [
          {
            _type: "hero",
            _version: 1,
            _key: "hero-1",
            heading: "A structured heading",
            body: "Useful article body text.",
            href: "https://links.example.test/skip-this-url",
          },
        ],
      },
    };

    const adapted = await adaptContentItemWithContext(item, contentApi, "en");
    const bodyText =
      adapted.document.body?.flatMap((block) =>
        block.children.map((child) => child.text),
      ) ?? [];

    expect(adapted.document.route).toBe("/journal/hello/");
    expect(adapted.document.body).toHaveLength(1);
    expect(bodyText.join(" ")).toContain("A structured heading");
    expect(bodyText.join(" ")).toContain("Useful article body text.");
    expect(bodyText.join(" ")).not.toContain("skip-this-url");
    expect(adapted.meta.publicUrl).toBe(
      "https://www.aexeo.test/journal/hello/",
    );
    expect(adapted.document.alternates).toEqual([
      { lang: "en", href: "https://www.aexeo.test/journal/hello/" },
      { lang: "fr", href: "https://www.aexeo.test/fr/journal/bonjour/" },
    ]);
  });

  it("uses a stable identity route for drafts and entries without public URLs", async () => {
    const contentApi: ContentUrlApi = {
      getPublicUrl: vi.fn(async () => null),
      getTranslations: vi.fn(async () => ({
        translationGroup: "draft-group",
        translations: [],
      })),
    };
    const item: EmdashContentItem = {
      id: "draft-123",
      type: "posts",
      slug: "draft-slug",
      status: "draft",
      locale: null,
      data: { title: "Draft" },
    };

    const adapted = await adaptContentItemWithContext(item, contentApi, "en");

    expect(adapted.document.route).toBe("/posts/draft-123");
    expect(adapted.document.lang).toBe("en");
    expect(adapted.meta.publicUrl).toBeNull();
    expect(contentApi.getPublicUrl).not.toHaveBeenCalled();
  });
});

describe("sandbox afterSave", () => {
  it("uses EmDash URL and translation APIs and evaluates the saved document", async () => {
    host = await createPluginRuntimeTestHost({
      site: {
        name: "Aexeo integration test",
        url: "https://www.aexeo.test",
        locale: "en",
        trailingSlash: "always",
      },
      i18n: { defaultLocale: "en", locales: ["en", "fr"] },
    });
    await host.fixtures.collection({
      slug: "posts",
      label: "Posts",
      urlPattern: "/journal/{slug}",
      routable: true,
      fields: [
        { slug: "title", label: "Title", type: "string" },
        { slug: "body", label: "Body", type: "portableText" },
      ],
    });
    const english = await host.fixtures.content("posts", {
      id: "article-en",
      slug: "hello",
      status: "published",
      locale: "en",
      data: { title: "Hello", body: body("Hello") },
    });
    await host.fixtures.content("posts", {
      id: "article-fr",
      slug: "bonjour",
      status: "published",
      locale: "fr",
      translationOf: english.id,
      data: { title: "Bonjour", body: body("Bonjour") },
    });
    await host.fixtures.plugin.kv(
      "config:evaluator_url",
      "https://evaluator.test",
    );
    await host.fixtures.plugin.kv("config:eval_token", "test-token");
    await host.http.respond(
      "https://evaluator.test/evaluate",
      new Response("[]", { status: 200 }),
    );

    const update = await host.actions.content.update("posts", english.id, {
      data: { title: "Hello updated", body: body("Hello updated") },
    });
    expect(update.success).toBe(true);

    const expectedPublicUrl = await host.inspect.content.publicUrl(
      "posts",
      english.id,
    );
    const expectedFrenchUrl = await host.inspect.content.publicUrl(
      "posts",
      "article-fr",
    );
    expect(expectedPublicUrl).toBe(
      "https://www.aexeo.test/journal/hello/",
    );
    if (expectedPublicUrl === null) {
      throw new Error("The EmDash test site did not resolve the public URL");
    }
    const documentKey = `document:${new URL(expectedPublicUrl).pathname}`;
    await vi.waitFor(async () => {
      expect(await host?.inspect.kv.get(documentKey)).not.toBeNull();
      expect(host?.http.requests()).toHaveLength(1);
    });
    const stored = await host.inspect.kv.get<{
      document: {
        route: string;
        body?: Array<{ children: Array<{ text: string }> }>;
        alternates?: Array<{ lang: string; href: string }>;
      };
      meta: { publicUrl: string | null };
    }>(documentKey);
    expect(stored?.document.route).toBe("/journal/hello/");
    expect(stored?.meta.publicUrl).toBe(expectedPublicUrl);
    expect(stored?.document.alternates).toContainEqual({
      lang: "fr",
      href: expectedFrenchUrl,
    });
    expect(
      stored?.document.body?.flatMap((block) =>
        block.children.map((child) => child.text),
      ).join(" "),
    ).toContain("Hello updated");

    const requests = host.http.requests();
    expect(requests[0]?.url).toBe("https://evaluator.test/evaluate");
    const requestBody = JSON.parse(
      new TextDecoder().decode(requests[0]!.body),
    ) as {
      documents: Array<{
        route: string;
        body?: Array<{ children: Array<{ text: string }> }>;
      }>;
    };
    expect(requestBody.documents[0]?.route).toBe("/journal/hello/");
    expect(
      requestBody.documents[0]?.body?.flatMap((block) =>
        block.children.map((child) => child.text),
      ).join(" "),
    ).toContain("Hello updated");
    // The findings entry is written after the document entry, so it is not
    // guaranteed to exist by the time the waits above pass. Poll for it
    // rather than asserting immediately, which made this assertion race the
    // hook's own writes and fail intermittently.
    await vi.waitFor(async () => {
      await expect(
        host.inspect.kv.get(`findings:/journal/hello/`),
      ).resolves.toMatchObject({ route: "/journal/hello/", findings: [] });
    });

    // Route migration. `persistDocument` re-keys a document whenever the
    // same content id appears under a new route, and deletes the stale
    // `document:` and `findings:` entries for the route it left behind.
    //
    // The stale entry is seeded directly rather than produced by editing the
    // entry's slug: @emdash-cms/plugin-test@0.2.4 accepts `slug` on
    // `actions.content.update`, reports `success: true`, and then silently
    // discards it, so the host cannot drive a real route change. Seeding the
    // previous route exercises exactly the same branch (the match keys off
    // `meta.id` + `meta.collection`, not off how the route changed).
    await host.fixtures.plugin.kv("document:/journal/previous/", {
      document: { route: "/journal/previous/" },
      meta: { id: english.id, collection: "posts", publicUrl: null },
    });
    await host.fixtures.plugin.kv("findings:/journal/previous/", {
      route: "/journal/previous/",
      findings: [],
    });
    expect(await host.inspect.kv.get("document:/journal/previous/")).not.toBeNull();

    await host.http.respond(
      "https://evaluator.test/evaluate",
      new Response("[]", { status: 200 }),
    );
    const moved = await host.actions.content.update("posts", english.id, {
      data: { title: "Hello moved", body: body("Hello moved") },
    });
    expect(moved.success).toBe(true);

    await vi.waitFor(async () => {
      expect(await host?.inspect.kv.get("document:/journal/previous/")).toBeNull();
      expect(
        await host?.inspect.kv.get("findings:/journal/previous/"),
      ).toBeNull();
      expect(await host?.inspect.kv.get(documentKey)).not.toBeNull();
      expect(host?.http.requests()).toHaveLength(2);
    });
    // The entry keeps serving its own current route; only the orphaned
    // keys for the route it left behind are removed.
    await expect(
      host.inspect.kv.get(documentKey),
    ).resolves.toMatchObject({
      document: { route: "/journal/hello/" },
      meta: { publicUrl: expectedPublicUrl },
    });
  });
});
