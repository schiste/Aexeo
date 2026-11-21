import type { SandboxCtx } from "./plugin.js";
import type { EmdashDocument } from "./types.js";

export const SETTINGS_LIST_STRUCTURE_KEY = "list_structure_complete_collections";

/** Only an explicit, well-formed host setting can attest a projected collection. */
export async function sandboxListCoverage(
  ctx: SandboxCtx, documents: readonly EmdashDocument[], collections?: readonly string[],
): Promise<readonly EmdashDocument[]> {
  let attested: readonly string[] = [];
  let status: "absent" | "configured" | "unavailable" = "absent";
  if (ctx.settings) {
    try {
      const raw = await ctx.settings.get<unknown>(SETTINGS_LIST_STRUCTURE_KEY);
      if (raw !== null && raw !== undefined && raw !== "") {
        if (typeof raw !== "string" || raw.length > 65536) throw new Error("invalid setting");
        const parsed: unknown = JSON.parse(raw);
        if (!Array.isArray(parsed) || !parsed.every((item) => typeof item === "string" && item.length > 0)) throw new Error("invalid setting");
        attested = parsed;
        status = "configured";
      }
    } catch {
      status = "unavailable";
      try { ctx.log?.warn?.("Aexeo list structure settings unavailable; list-dependent gates remain unverified"); } catch { /* advisory */ }
    }
  }
  let count = 0;
  const projected = documents.map((document, index) => {
    const collection = collections?.[index];
    if (typeof collection !== "string" || !attested.includes(collection)) return document;
    count++;
    return { ...document, list_structure_complete: true };
  });
  try { ctx.log?.info?.("aexeo.sandbox_list_coverage", { status, documents: documents.length, attested_documents: count }); } catch { /* advisory */ }
  return projected;
}
