// Real configured plugin and WASM, bundled by Wrangler just like a host Worker.
import { createPlugin } from "../../aexeo-emdash/dist/configured.js";

const plugin = createPlugin({ revisionAudits: true, contentPreset: { name: "article" } });
const current = { id: "one", type: "posts", locale: "en", draftRevisionId: "rev1", data: { content: "A useful paragraph." } };
const revision = { id: "rev1", collection: "posts", entryId: "one", data: current.data };
const values = new Map();
const ctx = {
  content: { get: async () => current, getRevision: async () => revision },
  kv: { get: async (key) => values.get(key), set: async (key, value) => values.set(key, value) },
  input: { collection: "posts", id: "one", revision: "rev1" },
};
export default { async fetch() {
  try { return Response.json(await plugin.routes["audit-revision"].handler(ctx)); }
  catch (error) { return Response.json({ error: String(error) }, { status: 500 }); }
} };
