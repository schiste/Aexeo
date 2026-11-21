// Drive the shipped sandbox hook and the real content WASM through a fake HTTP sidecar.
import assert from "node:assert/strict";
import plugin from "../dist/sandbox-entry.js";
import { evaluateContent } from "../dist/content.js";

const policy = `version=1
name="prose"
[[gates]]
id="variance"
kind="prose_sentence_length_variance"
window_size=3
min_stddev_words=2
`;
const block = (text, list = false) => ({ _type: "block", ...(list ? { listItem: "bullet", level: 1 } : {}), children: [{ _type: "span", text }] });
const body = [block("One two. Three four. Five six."), block("List one. List two. List three.", true), block("Seven eight. Nine ten. Eleven twelve.")];
const item = { id: "one", type: "posts", slug: "one", status: "draft", locale: "en", data: { body } };
const store = new Map([["config:evaluator_url", "https://example.test"]]);
let setting = null;
let lastReport;
const events = [];
const ctx = {
  kv: { get: async key => store.get(key) ?? null, set: async (key, value) => store.set(key, value), delete: async key => store.delete(key), list: async (prefix = "") => [...store].filter(([key]) => key.startsWith(prefix)).map(([key, value]) => ({ key, value })) },
  settings: { get: async key => key === "eval_token" ? "token" : key === "list_structure_complete_collections" ? setting : null, set: async () => {} },
  content: { list: async collection => ({ items: collection === "posts" ? [item] : [], hasMore: false }), getPublicUrl: async () => "https://example.test/one", getTranslations: async () => ({ translationGroup: "one", translations: [] }) },
  http: { fetch: async (_url, request) => {
    const payload = JSON.parse(request.body);
    const assessments = {};
    for (const document of payload.documents) {
      lastReport = await evaluateContent(document, "prose", { prose: policy });
      assessments[document.route] = { status: "evaluated", preset: "prose", gates: lastReport.gates };
    }
    return { ok: true, status: 200, text: async () => JSON.stringify({ findings: [], assessments, capabilities: ["observed_metadata_fields_v1"] }) };
  } },
  site: { url: "https://example.test", locale: "en" },
  log: { info: (name, data) => { if (name === "aexeo.sandbox_list_coverage") events.push(data); }, warn: () => {}, error: () => {} },
};
const save = () => plugin.hooks["content:afterSave"]({ content: item, collection: "posts", isNew: false }, ctx);
await save();
assert.equal(lastReport.gates[0].status, "not_checked");
setting = '["posts"]';
await save();
assert.equal(lastReport.gates[0].status, "failed");
assert.equal(lastReport.gates[0].measurements.list_paragraphs_excluded, 1);
assert.equal(lastReport.gates[0].measurements.windows_checked, 2);
await plugin.routes.admin.handler({ input: { type: "block_action", action_id: "refresh_findings" } }, ctx);
assert.equal(lastReport.gates[0].status, "failed");
item.data.body = [...body, { _type: "unknownComponent" }];
await save();
assert.equal(lastReport.gates[0].status, "not_checked");
assert.ok(events.some(event => event.attested_documents === 1));
assert.ok(!JSON.stringify(events).includes("posts"));
console.log("WASM sandbox save and refresh list coverage test passed");
