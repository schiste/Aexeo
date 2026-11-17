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

// Combining marks stay attached to their word in the configured CMS adapter.
{
  const preset = `version=1
name="terms"
[[gates]]
id="forbidden"
kind="forbidden_terms"
terms=["cafe"]
case_insensitive=true
`;
  const configured = createPlugin({ contentPreset: { name: "terms", sources: { terms: preset } } });
  const page = item("combining-marks", { data: { body: [block("p", "CAFE\u0301")] } });
  const host = fakeHost([page]);
  await configured.hooks["content:afterSave"].handler({ content: page, collection: "pages", isNew: false }, host.ctx);
  const gate = host.store.get("findings:/combining-marks").assessment.gates.find((item) => item.gate_id === "forbidden");
  assert.equal(gate.status, "passed");
  assert.equal(gate.measurements.matches, 0);
  assert.deepEqual(host.logged, []);
}

// Multilingual character bounds run through the configured host and real WASM.
{
  const preset = `version=1
name="characters"
[[gates]]
id="characters"
kind="character_count"
min=4
max=4
ignore_whitespace=true
`;
  const configured = createPlugin({ contentPreset: { name: "characters", sources: { characters: preset } } });
  const page = item("character-count", { data: { body: [block("p", "你好 世界")] } });
  const host = fakeHost([page]);
  const evaluate = () => configured.hooks["content:afterSave"].handler(
    { content: page, collection: "pages", isNew: false }, host.ctx,
  );
  await evaluate();
  let gate = host.store.get("findings:/character-count").assessment.gates.find((item) => item.gate_id === "characters");
  assert.equal(gate.status, "passed");
  assert.equal(gate.measurements.character_count, 4);
  page.data.body[0] = block("p", "你好");
  await evaluate();
  gate = host.store.get("findings:/character-count").assessment.gates.find((item) => item.gate_id === "characters");
  assert.equal(gate.status, "failed");
  assert.equal(gate.measurements.character_count, 2);
  page.data.body.push({ _type: "unknownComponent", _key: "unknown" });
  await evaluate();
  gate = host.store.get("findings:/character-count").assessment.gates.find((item) => item.gate_id === "characters");
  assert.equal(gate.status, "not_checked");
  assert.equal(gate.measurements?.character_count, undefined);
  assert.deepEqual(host.logged, []);
  console.log("WASM character count handles observed and incomplete content");
}

// Metadata bounds run through configured saves and the real WASM bridge.
{
  const preset = `version=1
name="metadata"
[[gates]]
id="title"
kind="metadata_length"
field="title"
max=3
`;
  const configured = createPlugin({ contentPreset: { name: "metadata", sources: { metadata: preset } } });
  const page = item("metadata-length");
  const host = fakeHost([page]);
  await configured.hooks["content:afterSave"].handler({ content: page, collection: "pages", isNew: false }, host.ctx);
  const gate = host.store.get("findings:/metadata-length").assessment.gates.find((item) => item.gate_id === "title");
  assert.equal(gate.status, "failed");
  assert.ok(gate.measurements.character_count > 3);
  assert.deepEqual(host.logged, []);
  console.log("WASM metadata length evaluates the configured title");
}

// The shipped preset must work through configured save and real WASM, with
// a source location an editor can use and a clean recheck after revision.
{
  const configured = createPlugin({ contentPreset: { name: "duplicate-paragraph-review" } });
  const paragraph = "The team checks backups before every planned deployment.";
  const page = item("duplicate-review", {
    data: { body: [block("one", paragraph), block("two", paragraph.toUpperCase())] },
  });
  const host = fakeHost([page]);
  const evaluate = () => configured.hooks["content:afterSave"].handler(
    { content: page, collection: "pages", isNew: false }, host.ctx,
  );
  await evaluate();
  const finding = host.store.get("findings:/duplicate-review").findings.find(
    (result) => result.rule_id === "editorial.duplicate_paragraphs",
  );
  assert.ok(finding);
  assert.deepEqual(finding.aexeo_cms_location,
    { path: ["data", "body", 1, "children", 0, "text"], utf16_offset: 0 });
  page.data.body[1] = block("two", "A different paragraph explains who owns recovery.");
  await evaluate();
  assert.equal(has(host, "/duplicate-review", "editorial.duplicate_paragraphs"), false);
  assert.deepEqual(host.logged, []);
  console.log("WASM duplicate paragraph preset reports and clears a CMS finding");
}

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
  const saved = host.store.get("findings:/external");
  assert.deepEqual(saved.findings.find((finding) => finding.rule_id === "external").aexeo_source_location,
    { block_index: 0, span_index: 0, utf16_offset: 0 });
  assert.deepEqual(saved.assessment.source_mapping, { eligible: 1, mapped: 1, unmapped: 0 });
  assert.deepEqual(saved.findings.find((finding) => finding.rule_id === "external").aexeo_cms_location,
    { path: ["data", "body", 0, "children", 0, "text"], utf16_offset: 0 });
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

// A failing provider must not discard unrelated editorial findings.
{
  const preset = `version=1
name="partial"
[[gates]]
id="words"
kind="word_count"
min=100
[[gates]]
id="external"
kind="custom"
extension="example.rule"
blocking=true
`;
  const telemetry = [];
  const configured = createPlugin({ contentPreset: { name: "partial", sources: { partial: preset } } }, {
    externalGates: async () => { throw new Error("private provider failure"); },
    onExternalGateTelemetry: (event) => telemetry.push(event),
  });
  const page = item("provider-failure");
  const host = fakeHost([page]);
  await configured.hooks["content:afterSave"].handler({ content: page, collection: "pages", isNew: false }, host.ctx);
  const saved = host.store.get("findings:/provider-failure");
  assert.ok(saved, "partial assessment was not persisted");
  assert.ok(saved.findings.some((finding) => finding.rule_id === "words"));
  assert.deepEqual(saved.assessment.gates.find((gate) => gate.gate_id === "external"), {
    gate_id: "external", status: "not_checked", blocking: true, scope: "document",
    required_capability: "extension", reason: "external gate provider failed",
  });
  assert.equal(telemetry[0].status, "error");
  assert.ok(!JSON.stringify(saved).includes("private provider failure"));
  assert.deepEqual(host.logged, []);
  console.log("WASM keeps local findings when an external provider fails");
}

// A configured provider must never receive documents whose resolved preset has
// no custom gate, on either refresh or save.
{
  let calls = 0;
  const configured = createPlugin({ contentPreset: { name: "article" } }, {
    externalGates: async () => { calls += 1; throw new Error("unexpected provider call"); },
  });
  const page = item("provider-not-needed");
  const host = fakeHost([page]);
  await configured.routes.refresh.handler(host.ctx);
  await configured.hooks["content:afterSave"].handler({ content: page, collection: "pages", isNew: false }, host.ctx);
  assert.equal(calls, 0);
  assert.deepEqual(host.logged, []);
  assert.ok(host.store.get("findings:/provider-not-needed"));
  console.log("WASM skips provider when the resolved preset has no custom gate");
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
    linkObservations: async ({ destinations, route, locale, preset: selected, collection }) => {
      calls.push({ destinations, route, locale, selected, collection });
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
  assert.equal(host.store.get("findings:/observed").assessment.gates[0].reason, "link observation failed");
  assert.equal(calls[0].route, "/observed");
  assert.equal(calls[1].route, "/observed");
  assert.equal(calls[0].locale, "en");
  assert.equal(calls[1].locale, "en");
  assert.ok(!JSON.stringify(host.store.get("findings:/observed")).includes("observer unavailable"));
  assert.equal(has(host, "/observed", "external-links"), false);
  assert.equal(calls.length, 2);
  assert.deepEqual(telemetry.map((event) => event.status), ["success", "error"]);
  assert.ok(!JSON.stringify(telemetry).includes("example.org"));
  console.log("WASM configured link provider runs on refresh and save");
}

// Evidence is selected separately for each CMS record, on refresh and save.
{
  const preset = `version=1
name="evidence"
[[gates]]
id="anchor"
kind="evidence_anchors"
source="study"
claim="Claim"
quote="Quote"
`;
  let fail = false;
  const calls = [];
  const telemetry = [];
  const configured = createPlugin({ contentPreset: { name: "custom", sources: { custom: preset }, evidenceSources: { study: "stale Quote" } } }, {
    evidenceSources: async ({ route, preset: selected, collection }) => {
      calls.push({ route, selected, collection });
      if (fail) throw new Error("private failure");
      return { study: route === "/evidence-a" ? "Quote" : "other" };
    },
    onEvidenceSourceTelemetry: (event) => telemetry.push(event),
  });
  const first = item("evidence-a", { data: { body: [block("p", "Claim")] } });
  const second = item("evidence-b", { data: { body: [block("p", "Claim")] } });
  const host = fakeHost([first, second]);
  await configured.routes.refresh.handler(host.ctx);
  assert.equal(host.store.get("findings:/evidence-a").assessment.gates[0].status, "passed");
  assert.equal(host.store.get("findings:/evidence-b").assessment.gates[0].status, "failed");
  fail = true;
  await configured.hooks["content:afterSave"].handler({ content: first, collection: "pages", isNew: false }, host.ctx);
  assert.equal(host.store.get("findings:/evidence-a").assessment.gates[0].status, "not_checked");
  assert.equal(calls.length, 3);
  assert.deepEqual(telemetry.map((event) => event.status), ["success", "success", "error"]);
  assert.ok(!JSON.stringify(telemetry).includes("private"));
  assert.deepEqual(host.logged, []);
  console.log("WASM document evidence provider runs on refresh and save");
}

// Locale selection applies the same way on whole-site refresh and individual saves.
{
  const events = [];
  const configured = createPlugin({ contentPreset: {
    name: "article", byCollection: { pages: "glossary" },
    byLocale: { "fr-fr": "article-readable" },
    byCollectionLocale: { pages: { "en-gb": "article-editorial" } },
  } }, { onPresetSelectionTelemetry: (event) => events.push(event) });
  const en = item("lang-en", { locale: "en-gb", data: { body: [block("p", "English text.")] } });
  const fr = item("lang-fr", { locale: "fr-fr", data: { body: [block("p", "Texte français.")] } });
  const ar = item("lang-ar", { locale: "ar-ae", data: { body: [block("p", "نص عربي.")] } });
  const host = fakeHost([en, fr, ar]);
  await configured.routes.refresh.handler(host.ctx);
  assert.equal(host.store.get("findings:/lang-en").assessment.preset, "article-editorial");
  assert.equal(host.store.get("findings:/lang-fr").assessment.preset, "article-readable");
  assert.equal(host.store.get("findings:/lang-ar").assessment.preset, "glossary");
  await configured.hooks["content:afterSave"].handler({ content: fr, collection: "pages", isNew: false }, host.ctx);
  assert.equal(host.store.get("findings:/lang-fr").assessment.preset, "article-readable");
  assert.deepEqual(events.map((event) => event.counts), [
    { collection_locale: 1, locale: 1, collection: 1, fallback: 0, none: 0 },
    { collection_locale: 0, locale: 1, collection: 0, fallback: 0, none: 0 },
  ]);
  assert.ok(!JSON.stringify(events).includes("fr-fr"));
  assert.deepEqual(host.logged, []);
  console.log("WASM configured presets select by collection and locale on refresh and save");
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
  assert.equal(host.store.get("findings:/mixed")?.body_coverage, "unavailable");
  for (const rule of ["CNT001", "GEO008", "GEO010"]) assert.equal(has(host, "/mixed", rule), false);
}

// An unrecognized conventional body field cannot certify another field as complete.
{
  const mixedFields = item("opaque-body", { data: { body: { document: "opaque" }, content: [block("a", "Preserved prose")] } });
  const host = fakeHost([mixedFields]);
  const configured = createPlugin({ contentPreset: { name: "article", collections: ["pages"] } });
  await configured.hooks["content:afterSave"].handler({ content: mixedFields, collection: "pages", isNew: false }, host.ctx);
  const document = host.store.get("document:/opaque-body").document;
  assert.equal(document.body[0].children[0].text, "Preserved prose");
  assert.deepEqual(document.body_capabilities, []);
  assert.equal(host.store.get("findings:/opaque-body").assessment.gates[0].status, "not_checked");
  for (const rule of ["CNT001", "GEO008", "GEO010"]) assert.equal(has(host, "/opaque-body", rule), false);
}

// A host can establish text without claiming unobserved component headings.
{
  const page = item("partial-capability", { data: { content: [block("a", "Before"), { _type: "card", _key: "c", text: "Card" }, block("b", "After")] } });
  const preset = `version=1
name="partial"
[[gates]]
id="words"
kind="word_count"
min=1
[[gates]]
id="headings"
kind="required_headings"
headings=["Summary"]
[[gates]]
id="phrase"
kind="forbidden_terms"
terms=["Before Card", "Card After"]
[[gates]]
id="sentence"
kind="sentence_length"
max_words=1
[[gates]]
id="paragraph"
kind="paragraph_count"
min=1
`;
  const configured = createPlugin({ contentPreset: { name: "partial", sources: { partial: preset } },
    blockMappings: { card: { fields: [{ kind: "paragraph", field: "text" }], capabilities: ["text"] } } });
  const host = fakeHost([page]);
  await configured.hooks["content:afterSave"].handler({ content: page, collection: "pages", isNew: false }, host.ctx);
  assert.deepEqual(host.store.get("document:/partial-capability").document.body_capabilities, ["text"]);
  const gates = host.store.get("findings:/partial-capability").assessment.gates;
  assert.equal(gates.find((gate) => gate.gate_id === "words").status, "passed");
  assert.equal(gates.find((gate) => gate.gate_id === "headings").status, "not_checked");
  assert.equal(gates.find((gate) => gate.gate_id === "phrase").status, "passed");
  assert.equal(gates.find((gate) => gate.gate_id === "sentence").status, "passed");
  assert.equal(gates.find((gate) => gate.gate_id === "paragraph").status, "not_checked");
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

// Unknown inline objects must not discard neighboring prose or certify coverage.
{
  const partial = item("inline-component", { data: { content: [{ _type: "block", children: [
    { _type: "span", text: "Before ", marks: [] },
    { _type: "widget", payload: "opaque" },
    { _type: "span", text: "after", marks: [] },
  ] }] } });
  const host = fakeHost([partial]);
  const configured = createPlugin({ contentPreset: { name: "article", collections: ["pages"] } });
  await configured.hooks["content:afterSave"].handler({ content: partial, collection: "pages", isNew: false }, host.ctx);
  const document = host.store.get("document:/inline-component").document;
  assert.deepEqual(document.body[0].children.map((child) => child.text), ["Before ", "after"]);
  assert.deepEqual(document.body_capabilities, []);
  assert.ok(host.store.get("findings:/inline-component").assessment.gates.some((gate) => gate.status === "not_checked"));
  for (const rule of ["CNT001", "GEO008", "GEO010"]) assert.equal(has(host, "/inline-component", rule), false);
}

// Malformed wire fields cannot discard neighboring prose or pass content gates.
{
  const page = item("malformed-spans", { data: { content: [{ _type: "block", style: "custom", markDefs: [{ _key: "bad", _type: "link", href: null }], children: [
    { _type: "span", text: "Before ", marks: null },
    { _type: "span", text: "after", marks: [1] },
  ] }] } });
  const host = fakeHost([page]);
  const configured = createPlugin({ contentPreset: { name: "article", collections: ["pages"] } });
  await configured.hooks["content:afterSave"].handler({ content: page, collection: "pages", isNew: false }, host.ctx);
  const document = host.store.get("document:/malformed-spans").document;
  assert.deepEqual(document.body[0].children.map((span) => span.text), ["Before ", "after"]);
  assert.deepEqual(document.body_capabilities, []);
  assert.ok(host.store.get("findings:/malformed-spans").assessment.gates.some((gate) => gate.status === "not_checked"));
  for (const rule of ["CNT001", "GEO008", "GEO010"]) assert.equal(has(host, "/malformed-spans", rule), false);
}

// Host-mapped inline content reaches the real evaluator in place.
{
  const page = item("mapped-inline", { data: { content: [{ _type: "block", children: [
    { _type: "span", text: "Read ", marks: [] },
    { _type: "inlineLink", label: "the guide", href: "/guide" },
    { _type: "span", text: " now.", marks: [] },
  ] }] } });
  const host = fakeHost([page]);
  const configured = createPlugin({ contentPreset: { name: "article", collections: ["pages"] },
    blockMappings: { inlineLink: { inline: [{ kind: "link", field: "label", hrefField: "href" }] } } });
  await configured.hooks["content:afterSave"].handler({ content: page, collection: "pages", isNew: false }, host.ctx);
  const doc = host.store.get("document:/mapped-inline").document;
  assert.deepEqual(doc.body[0].children.map((span) => span.text), ["Read ", "the guide", " now."]);
  assert.equal(doc.body[0].markDefs[0].href, "/guide");
  assert.equal(doc.body_capabilities, undefined);
  assert.ok(host.store.get("findings:/mapped-inline").assessment.gates.some((gate) => gate.status === "passed"));
}

// Original spans keep exact paths when a mapped inline component changes indices.
{
  const page = item("inline-locations", { data: { content: [{ _type: "block", children: [
    { _type: "span", text: "Alpha ", marks: [] },
    { _type: "inlineBadge", label: "Inserted ", href: "/target", title: "Linked" },
    { _type: "span", text: " Omega", marks: [] },
  ] }] } });
  const preset = `version=1
name="locations"
[[gates]]
id="terms"
kind="forbidden_terms"
terms=["Alpha", "Inserted", "Omega"]
blocking=false
`;
  const configured = createPlugin({ contentPreset: { name: "locations", sources: { locations: preset } },
    blockMappings: { inlineBadge: { inline: [{ kind: "text", field: "label" }, { kind: "link", field: "title", hrefField: "href" }] } } });
  const host = fakeHost([page]);
  const events = [];
  host.ctx.log.info = (name, data) => events.push([name, data]);
  await configured.hooks["content:afterSave"].handler({ content: page, collection: "pages", isNew: false }, host.ctx);
  const findings = host.store.get("findings:/inline-locations").findings.filter((finding) => finding.rule_id === "terms");
  assert.equal(findings.length, 3);
  assert.deepEqual(findings[0].aexeo_cms_location, { path: ["data", "content", 0, "children", 0, "text"], utf16_offset: 0 });
  assert.equal(findings[1].aexeo_cms_location, undefined);
  assert.deepEqual(findings[2].aexeo_cms_location, { path: ["data", "content", 0, "children", 2, "text"], utf16_offset: 1 });
  assert.deepEqual(events.find(([name]) => name === "aexeo.cms_location_coverage")?.[1], { eligible: 3, mapped: 2, unmapped: 1 });
  assert.equal(JSON.stringify(events).includes("Omega"), false);
}

// A selected but unobserved field cannot certify the remaining body as complete.
{
  const partial = item("missing-selected-field", { data: { introduction: [block("a", "Before")], conclusion: [block("b", "After")] } });
  const host = fakeHost([partial]);
  const configured = createPlugin({ contentPreset: { name: "article", collections: ["pages"] },
    contentFieldsByCollection: { pages: ["introduction", "sections", "conclusion"] } });
  await configured.hooks["content:afterSave"].handler({ content: partial, collection: "pages", isNew: false }, host.ctx);
  const document = host.store.get("document:/missing-selected-field").document;
  assert.deepEqual(document.body.map((entry) => entry.children[0].text), ["Before", "After"]);
  assert.deepEqual(document.body_capabilities, []);
  assert.ok(host.store.get("findings:/missing-selected-field").assessment.gates.some((gate) => gate.status === "not_checked"));
  for (const rule of ["CNT001", "GEO008", "GEO010"]) assert.equal(has(host, "/missing-selected-field", rule), false);
}

// Malformed optional mapping values cannot certify partial body content.
{
  const page = item("invalid-optional", { data: { blocks: [
    block("a", "Known prose"), { _type: "hero", _key: "h", _version: 1, title: "Visible title" },
  ] } });
  const host = fakeHost([page]);
  const configured = createPlugin({ contentPreset: { name: "article", collections: ["pages"] },
    blockMappings: { hero: [{ kind: "paragraph", field: "missing", optional: "false" }] } });
  const check = () => {
    const document = host.store.get("document:/invalid-optional").document;
    assert.equal(document.body[0].children[0].text, "Known prose");
    assert.deepEqual(document.body_capabilities, []);
    assert.ok(host.store.get("findings:/invalid-optional").assessment.gates.some((gate) => gate.status === "not_checked"));
    for (const rule of ["CNT001", "GEO008", "GEO010"]) assert.equal(has(host, "/invalid-optional", rule), false);
  };
  await configured.hooks["content:afterSave"].handler({ content: page, collection: "pages", isNew: false }, host.ctx);
  check();
  await configured.routes.refresh.handler(host.ctx);
  check();
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

// Component-only rich text works at both top level and inside a mapped section.
{
  const components = [{ _type: "notice", _key: "n", text: "First" }, { _type: "callout", _key: "c", text: "Second" }];
  const direct = item("components-direct", { data: { content: components } });
  const nested = item("components-nested", { data: { blocks: [{ _type: "section", _key: "s", body: components }] } });
  const host = fakeHost([direct, nested]);
  const mappings = { section: [{ kind: "richText", field: "body" }], notice: [{ kind: "heading", field: "text" }], callout: [{ kind: "paragraph", field: "text" }] };
  const configured = createPlugin({ contentPreset: { name: "article", collections: ["pages"] }, blockMappings: mappings });
  await configured.routes.refresh.handler(host.ctx);
  for (const route of ["/components-direct", "/components-nested"]) {
    const document = host.store.get(`document:${route}`).document;
    assert.deepEqual(document.body.map((entry) => entry.children[0].text), ["First", "Second"]);
    assert.equal(document.body[0].style, "h2");
    assert.equal(document.body_capabilities, undefined);
    assert.ok(host.store.get(`findings:${route}`).assessment.gates.some((gate) => gate.status === "passed"));
  }
  const partial = createPlugin({ contentPreset: { name: "article", collections: ["pages"] }, blockMappings: { section: mappings.section, notice: mappings.notice } });
  await partial.hooks["content:afterSave"].handler({ content: nested, collection: "pages", isNew: false }, host.ctx);
  assert.equal(host.store.get("document:/components-nested").document.body[0].style, "h2");
  assert.deepEqual(host.store.get("document:/components-nested").document.body_capabilities, []);
  assert.ok(host.store.get("findings:/components-nested").assessment.gates.some((gate) => gate.status === "not_checked"));
}

// The fallback budget cannot break JSON/WASM input at a Unicode boundary.
{
  const oversized = item("bounded-fallback", { data: { content: [block("a", "Before"),
    { _type: "unknown", _key: "u", text: "x".repeat(1_048_575) + "😀" }, block("b", "After")] } });
  const host = fakeHost([oversized]);
  const configured = createPlugin({ contentPreset: { name: "article", collections: ["pages"] } });
  await configured.hooks["content:afterSave"].handler({ content: oversized, collection: "pages", isNew: false }, host.ctx);
  const document = host.store.get("document:/bounded-fallback").document;
  assert.equal(document.body[1].children[0].text.length, 1_048_575);
  assert.equal(document.body[2].children[0].text, "After");
  assert.deepEqual(document.body_capabilities, []);
  assert.ok(host.store.get("findings:/bounded-fallback").assessment.gates.some((gate) => gate.status === "not_checked"));
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

// Required metadata never treats an unobserved description as missing.
{
  const preset = `version=1
name="metadata-test"
[[gates]]
id="description"
kind="required_metadata"
fields=["description"]
`;
  const unconfigured = createPlugin({ contentPreset: { name: "metadata-test", sources: { "metadata-test": preset } } });
  const legacy = item("unmapped-description");
  const legacyHost = fakeHost([legacy]);
  await unconfigured.hooks["content:afterSave"].handler({ content: legacy, collection: "pages", isNew: false }, legacyHost.ctx);
  assert.equal(legacyHost.store.get("findings:/unmapped-description").assessment.gates[0].status, "not_checked");
  const configured = createPlugin({ contentPreset: { name: "metadata-test", sources: { "metadata-test": preset } },
    descriptionFieldsByCollection: { pages: ["seo_description", "excerpt"] } });
  for (const [slug, data, status] of [
    ["unknown-description", { seo_description: null }, "not_checked"],
    ["absent-description", { seo_description: null, excerpt: null }, "failed"],
    ["present-description", { seo_description: "", excerpt: "Observed" }, "passed"],
  ]) {
    const page = item(slug, { data: { ...data, body: [block("p", "Content")] } });
    const host = fakeHost([page]);
    await configured.hooks["content:afterSave"].handler({ content: page, collection: "pages", isNew: false }, host.ctx);
    const result = host.store.get(`findings:/${slug}`);
    const gate = result.assessment.gates.find((gate) => gate.gate_id === "description");
    assert.equal(gate.status, status);
    assert.equal(gate.measurements.required_fields, 1);
    assert.equal(gate.measurements.present_fields, status === "passed" ? 1 : 0);
    assert.equal(gate.measurements.missing_fields, status === "failed" ? 1 : 0);
    assert.equal(gate.measurements.unavailable_fields, status === "not_checked" ? 1 : 0);
    assert.equal(result.findings.some((finding) => finding.rule_id === "description"), status === "failed");
  }
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
  ];
  for (const [route, ruleId] of expected) {
    assert.equal(has(host, route, ruleId), true, `fixture must trigger ${ruleId} on ${route} at Refresh; got ${[...rulesAt(host, route)]}`);
  }
  for (const ruleId of ["LNK001", "LNK002", "LNK004", "CNT005", "GEO001", "GEO007", "GEO009", "GEO011", "GEO012", "GEO013"]) {
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

// Quoted French prose must not produce phantom sentences or count guillemets as words.
{
  const preset = `version=1
name="quoted"
[[gates]]
id="sentence"
kind="sentence_length"
max_words=1
paragraphs_only=true
`;
  const configured = createPlugin({ contentPreset: { name: "quoted", sources: { quoted: preset } } });
  const page = item("quoted-french", { locale: "fr", data: { content: [block("p", "« Oui ! »")] } });
  const host = fakeHost([page]);
  await configured.hooks["content:afterSave"].handler({ content: page, collection: "pages", isNew: false }, host.ctx);
  assert.equal(host.store.get("findings:/quoted-french").assessment.gates[0].status, "passed");
  page.data.content[0].children[0].text = "« Oui ! » Long words.";
  await configured.hooks["content:afterSave"].handler({ content: page, collection: "pages", isNew: false }, host.ctx);
  const saved = host.store.get("findings:/quoted-french");
  assert.equal(saved.assessment.gates[0].status, "failed");
  const finding = saved.findings.find((entry) => entry.rule_id === "sentence");
  assert.deepEqual(finding.aexeo_source_location, { block_index: 0, span_index: 0, utf16_offset: 10 });
  assert.deepEqual(finding.aexeo_cms_location, { path: ["data", "content", 0, "children", 0, "text"], utf16_offset: 10 });
}

// Unsupported annotations may represent host-rendered links, so preserve
// prose but do not report link checks as complete.
{
  const preset = `version=1
name="annotation-coverage"
[[gates]]
id="words"
kind="word_count"
min=1
[[gates]]
id="links"
kind="link_destinations"
allowed_schemes=["https"]
allow_relative=true
`;
  const page = item("custom-annotation", { data: { body: [{
    _type: "block", children: [{ _type: "span", text: "Readable text", marks: ["internal"] }],
    markDefs: [{ _key: "internal", _type: "internalLink", target: "/destination" }],
  }] } });
  const host = fakeHost([page]);
  const configured = createPlugin({ contentPreset: { name: "annotation", sources: { annotation: preset } } });
  await configured.hooks["content:afterSave"].handler({ content: page, collection: "pages", isNew: false }, host.ctx);
  const stored = host.store.get("findings:/custom-annotation");
  assert.ok(stored?.assessment, "assessment should be persisted");
  assert.equal(stored.assessment.gates.find((gate) => gate.gate_id === "words").status, "passed");
  assert.equal(stored.assessment.gates.find((gate) => gate.gate_id === "links").status, "not_checked");
  page.data.body[0].markDefs = [{ _key: "internal", _type: "link", href: 12 }];
  await configured.hooks["content:afterSave"].handler({ content: page, collection: "pages", isNew: false }, host.ctx);
  const malformed = host.store.get("findings:/custom-annotation");
  assert.equal(malformed.assessment.gates.find((gate) => gate.gate_id === "words").status, "passed");
  assert.equal(malformed.assessment.gates.find((gate) => gate.gate_id === "links").status, "not_checked");
  assert.deepEqual(host.logged, []);
}
