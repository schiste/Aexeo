import { adaptContentItem, type CustomBlockMappings, type EmdashContentItem } from "./adapter.js";
import type { ContentPresetOptions } from "./content-evaluator.js";
import { selectContentPreset } from "./preset-selection.js";
import type { ContentReport } from "./content.js";
import type { SandboxCtx } from "./plugin.js";
import type { EmdashDocument } from "./types.js";

export interface RevisionAuditRequest { collection: string; id: string; revision: string }
interface Revision { id: string; collection: string; entryId: string; data: Record<string, unknown> }
type Current = EmdashContentItem & { draftRevisionId?: string | null; liveRevisionId?: string | null };
type RevisionContext = SandboxCtx & { content: SandboxCtx["content"] & {
  get?: (collection: string, id: string) => Promise<Current | null>;
  getRevision?: (collection: string, id: string, revision: string) => Promise<Revision | null>;
} };
export interface RevisionAuditOptions {
  preset?: ContentPresetOptions | undefined;
  blockMappings?: CustomBlockMappings | undefined;
  contentFieldsByCollection?: Record<string, readonly string[]> | undefined;
  descriptionFieldsByCollection?: Record<string, readonly string[]> | undefined;
  evidenceProviderConfigured?: boolean;
  evaluate: (document: EmdashDocument, preset: string, sources?: Record<string, string>, evidence?: Record<string, string>) => Promise<ContentReport>;
}

const reportKey = (id: string) => `revision-audit:${id}`;
const LIMITATION = "Only local configured content checks run; runtime providers are not invoked. Rendered page metadata and source freshness are not verified; the preset locale is taken from the current record; passing does not establish factual accuracy or currentness.";

/** Audit the exact host-provided historical data; never read or write article content. */
export async function auditRevision(ctx: RevisionContext, input: RevisionAuditRequest, options: RevisionAuditOptions) {
  const { collection, id, revision } = input;
  if (!ctx.content.get || !ctx.content.getRevision) return { status: "not_checked", reason: "revision_access_unavailable" } as const;
  const [current, snapshot] = await Promise.all([ctx.content.get(collection, id), ctx.content.getRevision(collection, id, revision)]);
  if (!current || !snapshot || snapshot.id !== revision || snapshot.collection !== collection || snapshot.entryId !== id) {
    return { status: "not_checked", reason: "revision_unavailable" } as const;
  }
  const selected = options.preset && (!options.preset.collections || options.preset.collections.includes(collection))
    ? selectContentPreset(options.preset, collection, current.locale ?? undefined) : undefined;
  if (!selected?.name) return { status: "not_checked", reason: "preset_not_configured", revision } as const;
  // A revision stores field data, not the historical SEO panel or rendered HTML.
  const item: EmdashContentItem = { id, type: collection, slug: null, status: "draft", locale: current.locale,
    data: snapshot.data };
  const adapted = adaptContentItem(item, { route: `/revision/${encodeURIComponent(id)}`, blockMappings: options.blockMappings,
    ...(options.contentFieldsByCollection?.[collection] === undefined ? {} : { contentFields: options.contentFieldsByCollection[collection] }),
    ...(options.descriptionFieldsByCollection?.[collection] === undefined ? {} : { descriptionFields: options.descriptionFieldsByCollection[collection] }) });
  // A runtime provider replaces static evidence in the ordinary evaluator.
  // It cannot safely be called for a historical revision without a revision-aware contract.
  const evidence = options.evidenceProviderConfigured ? undefined : options.preset?.evidenceSources;
  const report = await options.evaluate(adapted.document, selected.name, options.preset?.sources, evidence);
  const latest = await ctx.content.get(collection, id);
  const auditId = crypto.randomUUID();
  const result = { status: "evaluated" as const, audit_id: auditId, collection, id, revision,
    preset: selected.name, selection_source: selected.source, limitation: LIMITATION,
    source_paths: { blocks: adapted.meta.bodySourcePaths ?? null, spans: adapted.meta.bodySourceSpanPaths ?? null }, report };
  await ctx.kv.set(reportKey(auditId), result);
  return { ...result, revision_status: revisionStatus(latest, revision) };
}

function revisionStatus(current: Current | null, revision: string): "draft" | "published" | "stale" | "unknown" {
  if (!current || (current.draftRevisionId === undefined && current.liveRevisionId === undefined)) return "unknown";
  if (current.draftRevisionId === revision) return "draft";
  if (current.liveRevisionId === revision) return "published";
  return "stale";
}

/** Re-check status at read time; a stored report does not prove currentness. */
export async function getRevisionAudit(ctx: RevisionContext, auditId: string) {
  const stored = await ctx.kv.get<{ collection: string; id: string; revision: string }>(reportKey(auditId));
  if (!stored) return { status: "not_checked", reason: "audit_unavailable" } as const;
  const current = ctx.content.get ? await ctx.content.get(stored.collection, stored.id) : null;
  return { ...stored, revision_status: revisionStatus(current, stored.revision) };
}
