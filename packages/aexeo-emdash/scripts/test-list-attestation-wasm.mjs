import assert from "node:assert/strict";
import { createPlugin } from "../dist/configured.js";

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
const record = { id: "one", type: "posts", slug: "one", status: "draft", locale: "en", data: { intro: body, content: body } };
const base = { name: "prose", sources: { prose: policy }, fragmentFieldsByCollection: { posts: { intro: "prose" } }, listStructureCompleteByCollection: ["posts"] };
const store = new Map();
const revision = { id: "rev1", collection: "posts", entryId: "one", data: record.data };
const ctx = {
  content: { get: async () => record, getRevision: async () => revision,
    list: async () => ({ items: [record], hasMore: false }),
    getPublicUrl: async () => "https://example.org/one", getTranslations: async () => ({ translationGroup: "one", translations: [] }) },
  kv: { get: async key => store.get(key) ?? null, set: async (key, value) => store.set(key, value),
    delete: async key => store.delete(key), list: async () => [] },
  site: { url: "https://example.org", locale: "en" },
  log: { info: () => {}, warn: () => {}, error: () => {} },
};
const fragment = async (preset, item = record) => {
  const plugin = createPlugin({ contentPreset: { ...preset, autoAuditFragments: true } });
  await plugin.hooks["content:afterSave"].handler({ content: item, collection: "posts", isNew: false }, ctx);
  const saved = [...store.entries()].find(([key]) => key.startsWith("findings:"))?.[1];
  assert.ok(saved, `missing saved result; keys: ${[...store.keys()]}`);
  return saved.fragments[0].report.gates[0];
};
assert.equal((await fragment(base)).status, "not_checked");
const currentAttested = { ...base, fragmentListStructureCompleteByCollection: { posts: ["intro"] } };
const result = await fragment(currentAttested);
assert.equal(result.status, "failed");
assert.equal(result.measurements.windows_checked, 2);
assert.equal(result.measurements.list_paragraphs_excluded, 1);
assert.equal((await fragment(currentAttested, { ...record, data: { ...record.data, intro: [...body, { _type: "unknownComponent" }] } })).status, "not_checked");

const request = { collection: "posts", id: "one", revision: "rev1" };
const audit = async (preset, field) => {
  const plugin = createPlugin({ revisionAudits: true, contentFieldsByCollection: { posts: ["content"] }, contentPreset: preset });
  const result = await plugin.routes["audit-revision"].handler({ ...ctx, input: { ...request, ...(field ? { field } : {}) } });
  return result.report.gates[0];
};
assert.equal((await audit(currentAttested)).status, "not_checked");
assert.equal((await audit(currentAttested, "intro")).status, "not_checked");
assert.equal((await audit({ ...base, revisionListStructureCompleteByCollection: ["posts"] })).status, "failed");
assert.equal((await audit({ ...base, revisionFragmentListStructureCompleteByCollection: { posts: ["intro"] } }, "intro")).status, "failed");
assert.equal((await audit({ ...base, revisionFragmentListStructureCompleteByCollection: { posts: ["other"] } }, "intro")).status, "not_checked");
revision.data = { ...revision.data, intro: [...body, { _type: "unknownComponent" }] };
assert.equal((await audit({ ...base, revisionFragmentListStructureCompleteByCollection: { posts: ["intro"] } }, "intro")).status, "not_checked");
console.log("WASM fragment and revision scoped list attestation test passed");
