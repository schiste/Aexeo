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

// The configured host attests list structure only for its chosen collection.
{
  const preset = `version=1
name="prose"
[[gates]]
id="variance"
kind="prose_sentence_length_variance"
window_size=3
min_stddev_words=2
`;
  const events = [];
  const configured = createPlugin({ contentPreset: { name: "prose", sources: { prose: preset }, listStructureCompleteByCollection: ["pages"] } }, { onContentEvaluationTelemetry: event => events.push(event) });
  const page = item("prose-lists", { data: { body: [block("a", "One two. Three four. Five six."), { ...block("list", "List one. List two. List three."), listItem: "bullet", level: 1 }, block("b", "Seven eight. Nine ten. Eleven twelve.")] } });
  const host = fakeHost([page]);
  const evaluate = () => configured.hooks["content:afterSave"].handler({ content: page, collection: "pages", isNew: false }, host.ctx);
  await evaluate();
  let gate = host.store.get("findings:/prose-lists").assessment.gates.find((item) => item.gate_id === "variance");
  assert.equal(gate.status, "failed");
  assert.equal(gate.measurements.windows_checked, 2);
  assert.equal(gate.measurements.list_paragraphs_excluded, 1);
  assert.deepEqual(events[0].prose_variance, { measured_gates: 1, list_paragraphs_excluded: 1, windows_checked: 2 });
  assert.ok(!JSON.stringify(events[0]).includes("prose-lists"));
  page.data.body.push({ _type: "unknownComponent", _key: "unknown" });
  await evaluate();
  gate = host.store.get("findings:/prose-lists").assessment.gates.find((item) => item.gate_id === "variance");
  assert.equal(gate.status, "not_checked");
  assert.equal(events[1].prose_variance, undefined);
  assert.deepEqual(host.logged, []);
  console.log("WASM configured prose variance honors list and body completeness");
}

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

// External results are tied to the exact reference label seen by the provider.
{
  const preset = `version=1
name="custom-reference"
[[gates]]
id="external"
kind="custom"
extension="example.rule"
`;
  let firstInput;
  const plugin = createPlugin({ contentPreset: { name: "custom-reference", sources: { "custom-reference": preset } } }, {
    externalGates: async ({ input }) => {
      firstInput ??= input;
      return [{ extension: "example.rule", parameters: {}, input: firstInput, diagnostics: [] }];
    },
  });
  const page = item("external-reference", { data: { body: [{
    _type: "block", _key: "p", style: "normal",
    markDefs: [{ _key: "ref", _type: "link", href: "#reference-source" }],
    children: [{ _type: "span", text: "Élan." }, { _type: "span", text: "[1]", marks: ["ref"] }],
  }] } });
  const host = fakeHost([page]);
  const save = () => plugin.hooks["content:afterSave"].handler({ content: page, collection: "pages", isNew: false }, host.ctx);
  await save();
  assert.equal(firstInput.text, "Élan.[1]");
  assert.equal(host.store.get("findings:/external-reference").assessment.gates[0].status, "passed");
  page.data.body[0].children[1].text = "[2]";
  await save();
  assert.equal(host.store.get("findings:/external-reference").assessment.gates[0].status, "not_checked");
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
  const expectedComponents = { entries: [
    { kind: "block", type: "askYourAi", occurrences: 1 },
    { kind: "block", type: "unknown", occurrences: 1 },
  ], truncated: false };
  assert.deepEqual(host.store.get("findings:/mixed")?.body_unmapped_components, expectedComponents);
  for (const rule of ["CNT001", "GEO008", "GEO010"]) assert.equal(has(host, "/mixed", rule), false);
  await refresh(host);
  assert.deepEqual(host.store.get("findings:/mixed")?.body_unmapped_components, expectedComponents);
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

// Title fallbacks are useful display labels, not observed CMS title metadata.
{
  const preset = `version=1
name="title-test"
[[gates]]
id="title"
kind="required_metadata"
fields=["title"]
`;
  const configured = createPlugin({ contentPreset: { name: "title-test", sources: { "title-test": preset } } });
  for (const [slug, title, status] of [
    ["unknown-title", undefined, "not_checked"],
    ["empty-title", "", "failed"],
    ["present-title", "Observed title", "passed"],
  ]) {
    const page = item(slug, { seo: undefined, data: { ...(title === undefined ? {} : { title }), body: [block("p", "Content")] } });
    const host = fakeHost([page]);
    await configured.hooks["content:afterSave"].handler({ content: page, collection: "pages", isNew: false }, host.ctx);
    const result = host.store.get(`findings:/${slug}`);
    assert.equal(result.assessment.gates.find((gate) => gate.gate_id === "title").status, status);
    assert.deepEqual(host.logged, []);
  }
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
  page.data.body[0]._key = 42;
  page.data.body[0].children[0]._key = false;
  page.data.body[0].markDefs = [{ _key: "internal", _type: "link", href: "/destination" }];
  await configured.hooks["content:afterSave"].handler({ content: page, collection: "pages", isNew: false }, host.ctx);
  const badKeys = host.store.get("findings:/custom-annotation");
  assert.equal(badKeys.assessment.gates.find((gate) => gate.gate_id === "words").status, "passed");
  assert.equal(badKeys.assessment.gates.find((gate) => gate.gate_id === "links").status, "passed");
  assert.deepEqual(host.logged, []);
}

// Numeric citation labels remain available to literal gates without changing sentence rhythm.
{
  const preset = `version=1
name="citation"
[[gates]]
id="required"
kind="required_terms"
terms=["[17]"]
[[gates]]
id="length"
kind="sentence_length"
max_words=20
`;
  const page = item("literal-reference", { data: { body: [{
    _type: "block", _key: "p", style: "normal",
    markDefs: [{ _key: "ref", _type: "link", href: "#reference-source" }],
    children: [{ _type: "span", text: "Élan." }, { _type: "span", text: "[17]", marks: ["ref"] },
      { _type: "span", text: " Next sentence." }],
  }] } });
  const host = fakeHost([page]);
  const configured = createPlugin({ contentPreset: { name: "citation", sources: { citation: preset } } });
  await configured.hooks["content:afterSave"].handler({ content: page, collection: "pages", isNew: false }, host.ctx);
  const assessment = host.store.get("findings:/literal-reference").assessment;
  assert.equal(assessment.gates.find((gate) => gate.gate_id === "required").status, "passed");
  assert.equal(assessment.gates.find((gate) => gate.gate_id === "length").measurements.sentence_count, 2);
  assert.deepEqual(host.logged, []);
}

// Pacing binds the supplied structured snapshot to the CMS text projection.
{
  const preset = `version=1
name="pacing"
[[gates]]
id="editorial.pacing"
kind="editorial_pacing"
source="composition"
projection_separator="\\n"
profile={version=1,name="test",fields=[{name="hook",max_characters=3},{name="context"}]}
`;
  let structured = { hook: "four", context: "context" };
  const configured = createPlugin(
    { contentPreset: { name: "pacing", sources: { pacing: preset } } },
    { evidenceSources: async () => ({ composition: JSON.stringify(structured) }) },
  );
  const page = item("pacing", { data: { body: [block("p", "four"), block("p", "context")] } });
  const host = fakeHost([page]);
  const evaluate = () => configured.hooks["content:afterSave"].handler(
    { content: page, collection: "pages", isNew: false }, host.ctx,
  );
  const status = () => host.store.get("findings:/pacing").assessment.gates[0].status;
  await evaluate();
  assert.equal(status(), "failed");
  page.data.body = [block("p", "ok"), block("p", "context")];
  await evaluate();
  assert.equal(status(), "not_checked");
  structured = { hook: "ok", context: "context" };
  await evaluate();
  assert.equal(status(), "passed");
  assert.deepEqual(host.logged, []);
  console.log("WASM editorial pacing binds the complete CMS projection");
}

// Exercise the public revision-audit and review handlers against the real WASM.
// The host is simulated and no CMS content is modified.
const reviewStore = new Map();
const reviewCurrent = { id: "review-one", type: "posts", slug: "article", status: "draft", locale: "en", draftRevisionId: "rev1", liveRevisionId: null, data: { body: "current" } };
const reviewRevision = { id: "rev1", collection: "posts", entryId: "review-one", data: { body: [block("review", "A historical sentence.")] } };
const reviewContext = { kv: { get: async key => reviewStore.get(key), set: async (key, value) => { reviewStore.set(key, value); } }, content: { get: async () => reviewCurrent, getRevision: async () => reviewRevision } };
const reviewPreset = "version=1\nname='revision-review'\n[[gates]]\nid='length'\nkind='sentence_length'\nmax_words=1\n";
const reviewPlugin = createPlugin({ revisionAudits: true, contentPreset: { name: "revision-review", sources: { "revision-review": reviewPreset } } });
const revisionAudit = await reviewPlugin.routes["audit-revision"].handler({ ...reviewContext, input: { collection: "posts", id: "review-one", revision: "rev1" } });
assert.equal(revisionAudit.status, "evaluated");
assert.equal(revisionAudit.revision_status, "draft");
assert.ok(revisionAudit.report.gates[0].findings.length);
const reviewInput = { audit_id: revisionAudit.audit_id, gate_id: "length", finding_index: 0, decision: "retain_with_explanation", explanation: "Reviewed in context" };
assert.equal((await reviewPlugin.routes["set-revision-review"].handler({ ...reviewContext, input: reviewInput })).status, "recorded");
assert.equal((await reviewPlugin.routes["get-revision-review"].handler({ ...reviewContext, input: reviewInput })).review.revision, "rev1");
reviewCurrent.draftRevisionId = "rev2";
assert.equal((await reviewPlugin.routes["get-revision-audit"].handler({ ...reviewContext, input: { audit_id: revisionAudit.audit_id } })).revision_status, "stale");
console.log("WASM revision audit and review remain bound when the CMS revision changes");

// CMS-mapped metadata reaches real WASM on both save and refresh.
{
  const preset = `version=1
name="mapped"
[[gates]]
id="author"
kind="required_metadata"
fields=["author"]
[[gates]]
id="review-age"
kind="metadata_age"
field="last_reviewed"
as_of="2024-03-01"
max_days=1
[[gates]]
id="unmapped"
kind="required_metadata"
fields=["unmapped"]
`;
  const configured = createPlugin({
    contentPreset: { name: "mapped", sources: { mapped: preset } },
    metadataFieldsByCollection: { pages: { author: { source: "data", path: ["editorial", "author"] }, last_reviewed: { source: "data", path: ["editorial", "reviewed"] } } },
  });
  const page = item("mapped-metadata", { data: { body: [block("p", "Text")], editorial: { author: "Author", reviewed: "2024-02-28" } } });
  const host = fakeHost([page]);
  const events = [];
  host.ctx.log.info = (name, data) => events.push({ name, data });
  const status = () => host.store.get("findings:/mapped-metadata").assessment.gates[0].status;
  await configured.hooks["content:afterSave"].handler({ content: page, collection: "pages", isNew: false }, host.ctx);
  assert.equal(status(), "passed");
  const age = () => host.store.get("findings:/mapped-metadata").assessment.gates[1];
  assert.equal(age().status, "failed");
  assert.equal(age().measurements.age_days, 2);
  assert.equal(host.store.get("findings:/mapped-metadata").assessment.gates[2].status, "not_checked");
  assert.equal(host.store.get("findings:/mapped-metadata").findings.filter((finding) => finding.rule_id === "unmapped").length, 0);
  page.data.editorial = {};
  await configured.routes.refresh.handler(host.ctx);
  assert.equal(status(), "not_checked");
  assert.equal(age().status, "not_checked");
  page.data.editorial.author = null;
  await configured.hooks["content:afterSave"].handler({ content: page, collection: "pages", isNew: false }, host.ctx);
  assert.equal(status(), "failed");
  assert.ok(events.some((event) => event.name === "aexeo.metadata_projection" && event.data.supplied_fields === 2 && event.data.coverage_documents === 1 && event.data.observed_fields >= 3));
  assert.ok(events.some((event) => event.name === "aexeo.metadata_projection" && event.data.unavailable_fields === 2));
  assert.ok(!JSON.stringify(events).includes("Author"));
  console.log("WASM configured metadata distinguishes supplied, unavailable and absent");
}

// Automatic fragment audits remain separate from the whole page on save and refresh.
{
  const article = `version=1
name="article"
[[gates]]
id="article_words"
kind="word_count"
min=1
`;
  const fragment = `version=1
name="fragment"
[[gates]]
id="fragment_forbidden"
kind="forbidden_terms"
terms=["risk"]
[[gates]]
id="fragment_title"
kind="metadata_length"
field="title"
max=1
`;
  const telemetry = [];
  const configured = createPlugin({ contentPreset: {
    name: "article", sources: { article, fragment }, autoAuditFragments: true,
    fragmentFieldsByCollection: { pages: { intro: "fragment", missing: "fragment" } },
  } }, { onContentEvaluationTelemetry: event => telemetry.push(event) });
  const page = item("fragment-save", { data: {
    body: [block("body", "The complete article body.")],
    intro: [block("intro", "A risk needs attention.")],
  } });
  const host = fakeHost([page]);
  const runSave = () => configured.hooks["content:afterSave"].handler({ content: page, collection: "pages", isNew: false }, host.ctx);
  await runSave();
  assert.equal(telemetry.length, 2);
  assert.ok(!JSON.stringify(telemetry).includes("fragment-save"));
  let stored = host.store.get("findings:/fragment-save");
  assert.ok(stored.fragments, JSON.stringify({ stored, logged: host.logged }));
  assert.equal(stored.fragments[0].status, "evaluated");
  assert.equal(stored.fragments[0].report.gates[0].status, "failed");
  assert.equal(stored.fragments[0].report.gates[1].status, "not_checked");
  assert.deepEqual(stored.fragments[0].findings[0].aexeo_cms_location, { path: ["data", "intro", 0, "children", 0, "text"], utf16_offset: 2 });
  assert.equal(stored.fragments[1].reason, "fragment_field_unavailable");
  assert.ok(!stored.findings.some((f) => f.rule_id === "fragment_forbidden"));
  page.data.intro = [block("intro", "A safer introduction.")];
  await runSave();
  stored = host.store.get("findings:/fragment-save");
  assert.equal(stored.fragments[0].report.gates[0].status, "passed");
  await configured.routes.refresh.handler(host.ctx);
  stored = host.store.get("findings:/fragment-save");
  assert.equal(stored.fragments[0].report.gates[0].status, "passed");
  assert.equal(stored.fragments[1].status, "not_checked");
  assert.deepEqual(host.logged, []);
  console.log("WASM configured fragment audits persist separately on save and refresh");
}

// Current fragment observations are separately opted in and bound to the field.
{
  const policy = `version=1
name="fragment-links"
[[gates]]
id="links"
kind="link_reachability"
`;
  const page = item("fragment-links", { data: { body: [block("body", "Other content")], intro: [block("intro", "A source", "normal", "https://example.org/fragment")] } });
  const host = fakeHost([page]);
  const calls = [];
  const events = [];
  const configured = createPlugin({ contentFieldsByCollection: { pages: ["body"] }, contentPreset: {
    name: "article", autoAuditFragments: true, sources: { "fragment-links": policy },
    fragmentFieldsByCollection: { pages: { intro: "fragment-links" } },
  } }, { fragmentLinkObservations: async request => { calls.push(request); return ["not_found"]; }, onLinkProviderTelemetry: event => events.push(event) });
  await configured.hooks["content:afterSave"].handler({ content: page, collection: "pages", isNew: false }, host.ctx);
  const fragment = host.store.get("findings:/fragment-links")?.fragments?.[0];
  assert.equal(fragment?.report.gates[0].status, "failed");
  assert.deepEqual(fragment.findings[0].aexeo_cms_location?.path.slice(0, 2), ["data", "intro"]);
  assert.deepEqual(calls[0].destinations, ["https://example.org/fragment"]);
  assert.equal(calls[0].field, "intro");
  assert.equal(events[0].status, "success");
  assert.ok(!JSON.stringify(events).includes("example.org"));
}
