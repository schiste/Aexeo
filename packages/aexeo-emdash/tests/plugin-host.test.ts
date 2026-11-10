import {
  createPluginRuntimeTestHost,
  type PluginRuntimeTestHost,
} from "@emdash-cms/plugin-test";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  adaptContentItem,
  adaptContentItemWithContext,
  contentItemToEmdashDocument,
  type ContentUrlApi,
  type EmdashContentItem,
} from "../src/adapter.js";
import { buildAllowedHosts, buildCapabilities } from "../src/plugin.js";
import { aexeoPlugin } from "../src/index.js";

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
  it("distinguishes an absent body from an explicitly empty one without hiding another rich text field", () => {
    const item: EmdashContentItem = { id: "a", type: "posts", slug: "a", status: "draft", locale: "en", data: {} };
    expect(contentItemToEmdashDocument(item).body).toBeUndefined();
    expect(contentItemToEmdashDocument({ ...item, data: { body: "plain text" } }).body_capabilities).toEqual(["text"]);
    expect(contentItemToEmdashDocument({ ...item, data: { body: [] } }).body).toEqual([]);
    expect(contentItemToEmdashDocument({ ...item, data: { body: "  " } }).body).toEqual([]);
    expect(contentItemToEmdashDocument({ ...item, data: { body: [], content: body("actual text") } }).body?.[0]?.children[0]?.text).toBe("actual text");
    const multiField = contentItemToEmdashDocument({ ...item, data: { introduction: body("intro"), sections: [{ _type: "faq", _key: "f", _version: 1 }] } });
    expect(multiField.body_capabilities).toEqual([]);
    const selected = adaptContentItem({ ...item, data: { introduction: body("intro"), sections: body("section") } }, { contentFields: ["introduction", "sections"] }).document;
    expect(selected.body_capabilities).toBeUndefined();
    expect(selected.body?.map((block) => block.children[0]?.text)).toEqual(["intro", "section"]);
    expect(adaptContentItem(item, { contentFields: ["missing"] }).document.body).toBeUndefined();
    expect(adaptContentItem({ ...item, data: { introduction: body("intro"), sections: 42 } }, { contentFields: ["introduction", "sections"] }).document.body_capabilities).toEqual([]);
  });

  it("projects explicit ordered description fields and distinguishes unknown mappings", () => {
    const item: EmdashContentItem = { id: "a", type: "posts", slug: "a", status: "draft", locale: "en", data: { seo_description: "SEO copy", excerpt: "Excerpt" } };
    const options = { descriptionFields: ["seo_description", "excerpt"] };
    expect(adaptContentItem(item, options).document).toMatchObject({ description: "SEO copy", description_source_observed: true });
    expect(adaptContentItem({ ...item, data: { seo_description: "", excerpt: "Excerpt" } }, options).document).toMatchObject({ description: "Excerpt", description_source_observed: true });
    const absent = adaptContentItem({ ...item, data: {} }, options).document;
    expect(absent.description).toBeUndefined();
    expect(absent.description_source_observed).toBe(true);
    expect(adaptContentItem(item).document.description_source_observed).toBeUndefined();
    expect(adaptContentItem({ ...item, data: { seo_description: 42 } }, options).document.description_source_observed).toBeUndefined();
    expect(JSON.parse(JSON.stringify(aexeoPlugin({ descriptionFieldsByCollection: { posts: options.descriptionFields } }).options)).descriptionFieldsByCollection.posts).toEqual(options.descriptionFields);
  });

  it("preserves mixed Portable Text and marks unmapped components unverified", () => {
    const item: EmdashContentItem = { id: "a", type: "posts", slug: "a", status: "published", locale: "en", data: {
      content: [...body("Before"), { _type: "askYourAi", _key: "ai", prompt: "Question" }, ...body("After")],
    } };
    const unknown = contentItemToEmdashDocument(item);
    expect(unknown.body_capabilities).toEqual([]);
    expect(unknown.body?.map((block) => block.children[0]?.text)).toEqual(["Before", "Question", "After"]);
    const ignored = contentItemToEmdashDocument(item, { askYourAi: [] });
    expect(ignored.body_capabilities).toBeUndefined();
    expect(ignored.body?.map((block) => block.children[0]?.text)).toEqual(["Before", "After"]);
    const mapped = contentItemToEmdashDocument(item, { askYourAi: [{ kind: "paragraph", field: "prompt" }] });
    expect(mapped.body_capabilities).toBeUndefined();
    expect(mapped.body?.map((block) => block.children[0]?.text)).toEqual(["Before", "Question", "After"]);
  });

  it("retains valid mixed content around malformed items without claiming coverage", () => {
    const item: EmdashContentItem = { id: "a", type: "posts", slug: "a", status: "published", locale: "en", data: {
      content: [...body("Before"), null, { _type: "block", children: "broken" }, { prompt: "untyped" }, ...body("After")],
    } };
    const document = contentItemToEmdashDocument(item);
    expect(document.body?.map((block) => block.children[0]?.text)).toEqual(["Before", "After"]);
    expect(document.body_capabilities).toEqual([]);
  });

  it("retains later selected fields after an unrecognized body field", () => {
    const item: EmdashContentItem = { id: "a", type: "posts", slug: "a", status: "published", locale: "en", data: {
      introduction: body("Before"), custom: { unsupported: true }, conclusion: body("After"),
    } };
    const document = adaptContentItem(item, { contentFields: ["introduction", "custom", "conclusion"] }).document;
    expect(document.body?.map((block) => block.children[0]?.text)).toEqual(["Before", "After"]);
    expect(document.body_capabilities).toEqual([]);
  });

  it("declares only the current capabilities and configured sidecar host", () => {
    expect(buildCapabilities(null)).toEqual(["content:read"]);
    expect(buildAllowedHosts(null)).toEqual([]);
    expect(buildCapabilities("evaluator.test")).toEqual([
      "content:read",
      "network:request",
    ]);
    expect(buildAllowedHosts("evaluator.test")).toEqual(["evaluator.test"]);
  });

  it("maps declared custom block fields and rejects partial mappings", () => {
    const item: EmdashContentItem = { id: "a", type: "pages", slug: "a", status: "draft", locale: "en", data: { blocks: [{ _type: "hero", _key: "h", _version: 1, heading: "Title", body: "Body", label: "Read", href: "https://example.org" }] } };
    const mappings = { hero: [{ kind: "heading" as const, field: "heading" }, { kind: "paragraph" as const, field: "body" }, { kind: "link" as const, field: "label", hrefField: "href" }] };
    const document = contentItemToEmdashDocument(item, mappings);
    expect(document.body_capabilities).toBeUndefined();
    expect(document.body?.map((block) => block.children[0]?.text)).toEqual(["Title", "Body", "Read"]);
    expect(document.body?.[0]?.style).toBe("h2");
    expect(document.body?.[2]?.markDefs?.[0]?.href).toBe("https://example.org");
    expect(contentItemToEmdashDocument({ ...item, data: { blocks: [...(item.data.blocks as object[]), { _type: "unknown", _key: "u", _version: 1 }] } }, mappings).body_capabilities).toEqual([]);
    expect(contentItemToEmdashDocument(item, { hero: [{ kind: "paragraph", field: "missing" }] }).body_capabilities).toEqual([]);
    const faq = { ...item, data: { blocks: [{ _type: "faq", _key: "f", _version: 1, heading: "Questions", faqs: [{ question: "Why?", answer: "Because." }] }] } };
    const faqMapping = { faq: [{ kind: "heading" as const, field: "heading" }, { kind: "paragraph" as const, field: "subtitle", optional: true }, { kind: "repeat" as const, field: "faqs", fields: [{ kind: "heading" as const, field: "question", level: 3 as const }, { kind: "paragraph" as const, field: "answer" }] }] };
    const mappedFaq = contentItemToEmdashDocument(faq, faqMapping);
    expect(mappedFaq.body?.map((block) => block.children[0]?.text)).toEqual(["Questions", "Why?", "Because."]);
    expect(mappedFaq.body?.[1]?.style).toBe("h3");
    const badFaq = { ...faq, data: { blocks: [{ _type: "faq", _key: "f", _version: 1, heading: "Questions", faqs: ["bad"] }] } };
    expect(contentItemToEmdashDocument(badFaq, faqMapping).body_capabilities).toEqual([]);
    expect(contentItemToEmdashDocument({ ...item, data: { blocks: [{ _type: "hero", _key: "h", _version: 1, href: "/missing-label" }] } }, { hero: [{ kind: "link", field: "label", hrefField: "href", optional: true }] }).body_capabilities).toEqual([]);
    const rich = { ...item, data: { blocks: [{ _type: "hero", _key: "h", _version: 1, body: body("Nested") }] } };
    expect(contentItemToEmdashDocument(rich, { hero: [{ kind: "richText", field: "body" }] }).body?.[0]?.children[0]?.text).toBe("Nested");
    expect(JSON.parse(JSON.stringify(aexeoPlugin({ blockMappings: mappings }).options)).blockMappings).toEqual(mappings);
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
    expect(adapted.document.body_capabilities).toEqual([]);
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
      // `host` is module-scoped `let`, so the narrowing from the assignment
      // at the top of this test is lost inside a callback body. Re-narrow
      // locally rather than using `?.` here, which would turn a real
      // assertion into an `expect(undefined).resolves` that passes vacuously.
      const testHost = host;
      if (testHost === undefined) {
        throw new Error("The EmDash test host was disposed mid-test");
      }
      await expect(testHost.inspect.kv.get(`findings:/journal/hello/`)).resolves.toMatchObject({
        route: "/journal/hello/",
        findings: [],
      });
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

    // A distinctive finding proves that this second hook has completed its
    // final write before the test host and its database are disposed.
    const movedFinding = {
      rule_id: "moved", path: "/journal/hello/", message: "Moved",
      line: 1, column: 1, severity: "warning", suggestion: null, scope: "page",
    };
    await host.http.respond(
      "https://evaluator.test/evaluate",
      new Response(JSON.stringify([movedFinding]), { status: 200 }),
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
      expect(await host?.inspect.kv.get("findings:/journal/hello/")).toMatchObject({
        route: "/journal/hello/",
        findings: [movedFinding],
      });
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
