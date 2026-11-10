// Run after the plugin build. Drives the real configured plugin against the
// real WASM: only the CMS host (KV and content API) is faked, because a stub
// evaluator is what hid this bug. A save evaluates one document, so every rule
// that consults other documents sees a one-page site.
import assert from "node:assert/strict";
import { createPlugin } from "../dist/configured.js";

const SITE = "https://example.com";
const LONG_COPY = "Shared paragraph about how the robots are assembled and tested. ".repeat(40);

function block(key, text, style = "normal", href) {
  return {
    _type: "block",
    _key: key,
    style,
    markDefs: href === undefined ? [] : [{ _key: `${key}-link`, _type: "link", href }],
    children: [
      { _type: "span", text, marks: href === undefined ? [] : [`${key}-link`] },
    ],
  };
}

function item(slug, overrides = {}) {
  return {
    id: slug,
    type: "pages",
    slug,
    status: "published",
    locale: "en",
    seo: {
      title: `Title of ${slug}`,
      description: `Description of ${slug}.`,
      canonical: `${SITE}/${slug}`,
    },
    data: { body: [block("h", `Heading of ${slug}`, "h1")] },
    ...overrides,
  };
}

function fakeHost(items, translations = {}) {
  const store = new Map();
  const logged = [];
  return {
    store,
    items,
    logged,
    ctx: {
      kv: {
        get: async (key) => (store.has(key) ? store.get(key) : null),
        set: async (key, value) => void store.set(key, value),
        delete: async (key) => store.delete(key),
        list: async (prefix = "") =>
          [...store].filter(([key]) => key.startsWith(prefix)).map(([key, value]) => ({ key, value })),
      },
      content: {
        list: async (collection) => ({
          items: collection === "pages" ? items : [],
          hasMore: false,
        }),
        getPublicUrl: async (_collection, id) => `${SITE}/${id}`,
        getTranslations: async (_collection, id) => ({
          translationGroup: id,
          translations: translations[id] ?? [],
        }),
      },
      site: { url: SITE, locale: "en" },
      log: {
        info: () => {},
        warn: (message) => logged.push(message),
        error: (message, data) => logged.push(`${message} ${JSON.stringify(data)}`),
      },
    },
  };
}

const plugin = createPlugin();
const refresh = async (host) => {
  await plugin.routes.refresh.handler(host.ctx);
  assert.equal(host.logged.length, 0, `refresh logged errors: ${host.logged.join("; ")}`);
};
const save = (host, content) =>
  plugin.hooks["content:afterSave"].handler(
    { content, collection: "pages", isNew: false },
    host.ctx,
  );
const rulesAt = (host, route) => {
  const findings = host.store.get(`findings:${route}`)?.findings ?? [];
  return new Set(findings.map((finding) => finding.rule_id));
};
const has = (host, route, ruleId) => rulesAt(host, route).has(ruleId);

// Exercise the injected runtime provider through the actual configured hooks
// and WASM, including a later fix and re-evaluation.
{
  const preset = `version=1
name="custom"
[[gates]]
id="external"
kind="custom"
extension="example.rule"
blocking=false
[gates.parameters]
mode="strict"
`;
  const calls = [];
  let issue = true;
  const pluginWithProvider = createPlugin({ contentPreset: { name: "custom", sources: { custom: preset } } }, {
    externalGates: async ({ input, preset: selected, collection }) => {
      calls.push({ selected, collection, id: input.id });
      return [{ extension: "example.rule", parameters: { mode: "strict" }, input,
        diagnostics: issue ? [{ message: "Review the opening", line: 1, column: 1 }] : [] }];
    },
  });
  const page = item("external");
  const host = fakeHost([page]);
  await pluginWithProvider.routes.refresh.handler(host.ctx);
  assert.equal(has(host, "/external", "external"), true);
  issue = false;
  await pluginWithProvider.hooks["content:afterSave"].handler({ content: page, collection: "pages", isNew: false }, host.ctx);
  assert.equal(has(host, "/external", "external"), false);
  assert.deepEqual(calls, [
    { selected: "custom", collection: "pages", id: "/external" },
    { selected: "custom", collection: "pages", id: "/external" },
  ]);
  assert.deepEqual(host.logged, []);
  console.log("WASM configured provider runs on refresh and save");
}

// Explicit link observers run through both CMS workflows; provider errors
// preserve the prior report with a not-checked gate.
{
  const preset = `version=1
name="links"
[[gates]]
id="external-links"
kind="link_reachability"
`;
  let fail = false;
  const calls = [];
  const telemetry = [];
  const configured = createPlugin({ contentPreset: { name: "custom", sources: { custom: preset } } }, {
    linkObservations: async ({ destinations, preset: selected, collection }) => {
      calls.push({ destinations, selected, collection });
      if (fail) throw new Error("observer unavailable");
      return destinations.map(() => "not_found");
    },
    onLinkProviderTelemetry: (event) => telemetry.push(event),
  });
  const page = item("observed", { data: { body: [block("a", "Link", "normal", "https://example.org/missing")] } });
  const host = fakeHost([page]);
  await configured.routes.refresh.handler(host.ctx);
  assert.equal(has(host, "/observed", "external-links"), true);
  assert.equal(host.store.get("findings:/observed").assessment.gates[0].status, "failed");
  fail = true;
  await configured.hooks["content:afterSave"].handler({ content: page, collection: "pages", isNew: false }, host.ctx);
  assert.equal(host.store.get("findings:/observed").assessment.gates[0].status, "not_checked");
  assert.equal(has(host, "/observed", "external-links"), false);
  assert.equal(calls.length, 2);
  assert.deepEqual(telemetry.map((event) => event.status), ["success", "error"]);
  assert.ok(!JSON.stringify(telemetry).includes("example.org"));
  console.log("WASM configured link provider runs on refresh and save");
}

// --- A save must not invent findings from its one-page view -----------------

{
  const home = item("home", {
    data: {
      body: [
        block("h", "Heading of home", "h1"),
        block("p", "See our pricing page for the plans we offer", "normal", "/pricing"),
      ],
    },
  });
  const pricing = item("pricing", {
    data: {
      body: [
        block("h", "Heading of pricing", "h1"),
        block("p", "Back to the home page of the company", "normal", "/home"),
      ],
    },
  });
  const host = fakeHost([home, pricing]);
  await refresh(host);
  assert.equal(has(host, "/home", "LNK001"), false, "the link target exists on the site");
  assert.equal(has(host, "/pricing", "LNK002"), false, "/home links to /pricing");
  assert.equal(has(host, "/home", "LNK002"), false, "/pricing links to /home");

  await save(host, home);
  assert.equal(has(host, "/home", "LNK001"), false, "save reported a false broken link");
  assert.equal(has(host, "/home", "LNK002"), false, "save reported a false orphan");
  console.log("WASM save keeps valid links and inbound links clean");

  // CMS metadata is a projection, not evidence about the rendered page.
  const withoutDescription = { ...home, seo: { ...home.seo, description: null } };
  await save(host, withoutDescription);
  assert.equal(has(host, "/home", "SEO002"), false, "unobserved rendered metadata must not be reported missing");
  for (const rule of ["SEO004", "SEO005", "SOC001", "SOC002", "SOC003", "SOC004", "SOC006", "A11Y005", "CNT001", "GEO008", "GEO010"]) {
    assert.equal(has(host, "/home", rule), false, `${rule} requires rendered-page evidence`);
  }
  console.log("WASM CMS save does not report unobserved page elements");
}

// Mixed Portable Text retains prose on both sides of unrecognized components.
{
  const mixed = item("mixed", { data: { content: [block("a", "Before"), { _type: "askYourAi", _key: "ai", prompt: "Question" }, block("b", "Between"), { _type: "unknown", _key: "other" }, block("c", "After")] } });
  const host = fakeHost([mixed]);
  await save(host, mixed);
  const document = host.store.get("document:/mixed")?.document;
  assert.ok(document, "adapted document was persisted");
  assert.deepEqual(document.body.filter((entry) => ["a", "b", "c"].includes(entry._key)).map((entry) => entry.children[0].text), ["Before", "Between", "After"]);
  assert.deepEqual(document.body_capabilities, []);
  for (const rule of ["CNT001", "GEO008", "GEO010"]) assert.equal(has(host, "/mixed", rule), false);
}

// Malformed mixed items and unrecognized fields do not hide adjacent prose.
{
  const partial = item("partial-projection", { data: {
    introduction: [block("a", "Before"), null, { _type: "block", children: "broken" }, block("b", "Between")],
    unknown: { unexpected: true },
    conclusion: [block("c", "After")],
  } });
  const host = fakeHost([partial]);
  const configured = createPlugin({ contentPreset: { name: "article", collections: ["pages"] },
    contentFieldsByCollection: { pages: ["introduction", "unknown", "conclusion"] } });
  await configured.hooks["content:afterSave"].handler({ content: partial, collection: "pages", isNew: false }, host.ctx);
  const document = host.store.get("document:/partial-projection")?.document;
  assert.deepEqual(document.body.map((entry) => entry.children[0].text), ["Before", "Between", "After"]);
  assert.deepEqual(document.body_capabilities, []);
  assert.ok(host.store.get("findings:/partial-projection")?.assessment?.gates.some((gate) => gate.status === "not_checked"));
  for (const rule of ["CNT001", "GEO008", "GEO010"]) assert.equal(has(host, "/partial-projection", rule), false);
}

// Nested mixed rich text retains ordered blocks; unknown components stay unchecked.
{
  const nested = item("nested", { data: { blocks: [{ _type: "section", _key: "s", body: [
    block("a", "Before", "h2", "https://example.org/source"), { _type: "notice", _key: "n", text: "Middle" }, block("b", "After"),
  ] }] } });
  const host = fakeHost([nested]);
  const options = { contentPreset: { name: "article", collections: ["pages"] },
    blockMappings: { section: [{ kind: "richText", field: "body" }] } };
  const partial = createPlugin(options);
  await partial.hooks["content:afterSave"].handler({ content: nested, collection: "pages", isNew: false }, host.ctx);
  assert.deepEqual(host.store.get("document:/nested").document.body.map((entry) => entry.children[0].text), ["Before", "Middle", "After"]);
  assert.deepEqual(host.store.get("document:/nested").document.body_capabilities, []);
  assert.equal(host.store.get("document:/nested").document.body[0].style, "h2");
  assert.equal(host.store.get("document:/nested").document.body[0].markDefs[0].href, "https://example.org/source");
  assert.ok(host.store.get("findings:/nested").assessment.gates.some((gate) => gate.status === "not_checked"));
  const mapped = createPlugin({ ...options, blockMappings: { ...options.blockMappings, notice: [{ kind: "paragraph", field: "text" }] } });
  await mapped.routes.refresh.handler(host.ctx);
  assert.equal(host.store.get("document:/nested").document.body_capabilities, undefined);
  assert.ok(host.store.get("findings:/nested").assessment.gates.some((gate) => gate.status === "passed"));
}

// Ordered host metadata mappings reach both save and Refresh.
{
  const mapped = item("mapped", { data: { body: [block("p", "Content")], seo_description: "Preferred", excerpt: "Fallback" } });
  const host = fakeHost([mapped]);
  const configured = createPlugin({ descriptionFieldsByCollection: { pages: ["seo_description", "excerpt"] } });
  await configured.hooks["content:afterSave"].handler({ content: mapped, collection: "pages", isNew: false }, host.ctx);
  assert.equal(host.store.get("document:/mapped").document.description, "Preferred");
  assert.equal(host.store.get("document:/mapped").document.description_source_observed, true);
  host.items[0] = { ...mapped, data: { ...mapped.data, seo_description: "" } };
  await configured.routes.refresh.handler(host.ctx);
  assert.equal(host.store.get("document:/mapped").document.description, "Fallback");
}

// A partially projected body does not abort the preset or later documents.
{
  const mixed = item("partial", { data: { content: [block("a", "Before"), { _type: "askYourAi", _key: "ai" }, block("b", "After")] } });
  const complete = item("complete", { data: { content: [block("a", "Some content")] } });
  const host = fakeHost([mixed, complete]);
  const configured = createPlugin({ contentPreset: { name: "article", collections: ["pages"] }, contentFieldsByCollection: { pages: ["content"] } });
  await configured.routes.refresh.handler(host.ctx);
  const partial = host.store.get("findings:/partial");
  const good = host.store.get("findings:/complete");
  assert.ok(partial?.assessment?.gates.some((gate) => gate.status === "not_checked"));
  assert.ok(good?.assessment?.gates.some((gate) => gate.status === "passed"));
  const payload = await configured.routes.data.handler(host.ctx);
  assert.ok(payload.routes.find((route) => route.route === "/partial")?.assessment?.gates.some((gate) => gate.status === "not_checked"));
  assert.equal(host.logged.length, 0, `incomplete gates are not errors: ${host.logged.join("; ")}`);
}

// --- A save must not erase what Refresh found --------------------------------

// Observable site-graph findings survive a save of /subject. Conclusions about
// missing rendered targets and orphan pages are not established by CMS data.
{
  const subject = item("subject", {
    seo: { title: "Same title", description: "Same description.", canonical: `${SITE}/subject` },
    data: {
      body: [
        block("h", "Heading of subject", "h1"),
        block("copy", LONG_COPY),
        block("broken", "Read the missing page for details", "normal", "/missing"),
        block("weak", "click here", "normal", "/inbound"),
      ],
    },
  });
  const twin = item("twin", {
    seo: { title: "Same title", description: "Same description.", canonical: `${SITE}/twin` },
    data: subject.data,
  });
  const inbound = item("inbound");
  // /moved canonicalizes to a route that exists only in the whole-site view.
  const moved = item("moved", {
    seo: { title: "Moved", description: "Moved page.", canonical: `${SITE}/inbound` },
  });
  // /fr-subject exists but does not link back, so the cluster is not reciprocal.
  const translated = item("fr-subject", { locale: "fr" });
  const translations = {
    "fr-subject": [],
    subject: [
      { id: "subject", locale: "en", slug: "subject", status: "published", updatedAt: "" },
      { id: "fr-subject", locale: "fr", slug: "fr-subject", status: "published", updatedAt: "" },
    ],
    moved: [
      { id: "moved", locale: "en", slug: "moved", status: "published", updatedAt: "" },
      { id: "gone", locale: "fr", slug: "gone", status: "published", updatedAt: "" },
    ],
  };
  const host = fakeHost([subject, twin, inbound, moved, translated], translations);
  await refresh(host);

  const expected = [
    ["/subject", "LNK003"],
    ["/subject", "CNT005"],
  ];
  for (const [route, ruleId] of expected) {
    assert.equal(has(host, route, ruleId), true, `fixture must trigger ${ruleId} on ${route} at Refresh; got ${[...rulesAt(host, route)]}`);
  }
  for (const ruleId of ["LNK001", "LNK002", "LNK004"]) {
    assert.equal(has(host, "/subject", ruleId), false, `CMS-only Refresh must not assert ${ruleId}`);
  }

  for (const [route, content] of [["/subject", subject], ["/moved", moved], ["/inbound", inbound]]) {
    const before = rulesAt(host, route);
    await save(host, content);
    assert.deepEqual(
      [...rulesAt(host, route)].sort(),
      [...before].sort(),
      `saving unchanged ${route} must leave its findings as Refresh stored them`,
    );
  }
  console.log("WASM save preserves stored site-graph findings until Refresh");

  await refresh(host);
  for (const [route, ruleId] of expected) {
    assert.equal(has(host, route, ruleId), true, `Refresh must recompute ${ruleId} on ${route}`);
  }
}
