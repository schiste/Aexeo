import { adaptContentItem, type CustomBlockMappings, type DescriptionField, type MetadataFields, type EmdashContentItem } from "./adapter.js";
import type { ContentPresetOptions } from "./content-evaluator.js";
import { selectFragmentPreset } from "./fragment-selection.js";
import { selectContentPreset } from "./preset-selection.js";
import type { ContentReport } from "./content.js";
import type { SandboxCtx } from "./plugin.js";
import type { EmdashDocument } from "./types.js";
import { cmsBodyCoverage } from "./cms-coverage.js";

export interface RevisionAuditRequest { collection: string; id: string; revision: string; field?: string }
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
  descriptionFieldsByCollection?: Record<string, readonly DescriptionField[]> | undefined;
  metadataFieldsByCollection?: Record<string, MetadataFields> | undefined;
  evidenceProviderConfigured?: boolean;
  evaluate: (document: EmdashDocument, preset: string, sources?: Record<string, string>, evidence?: Record<string, string>) => Promise<ContentReport>;
}

const reportKey = (id: string) => `revision-audit:${id}`;
const LIMITATION = "Only local configured content checks run; runtime providers are not invoked. Rendered page metadata and source freshness are not verified; the preset locale is taken from the current record; passing does not establish factual accuracy or currentness and does not authorize publication.";

/** Audit the exact host-provided historical data; never read or write article content. */
export async function auditRevision(ctx: RevisionContext, input: RevisionAuditRequest, options: RevisionAuditOptions) {
  const { collection, id, revision } = input;
  if (!ctx.content.get || !ctx.content.getRevision) return { status: "not_checked", reason: "revision_access_unavailable" } as const;
  const [current, snapshot] = await Promise.all([ctx.content.get(collection, id), ctx.content.getRevision(collection, id, revision)]);
  if (!current || !snapshot || snapshot.id !== revision || snapshot.collection !== collection || snapshot.entryId !== id) {
    return { status: "not_checked", reason: "revision_unavailable" } as const;
  }
  const localeAtAudit = observedLocale(current.locale);
  const field = input.field;
  const fragmentPreset = field === undefined ? undefined : selectFragmentPreset(options.preset, collection, field);
  if (field !== undefined && !fragmentPreset) return { status: "not_checked", reason: "fragment_preset_not_configured", revision, field } as const;
  if (field !== undefined && !Object.hasOwn(snapshot.data, field)) return { status: "not_checked", reason: "fragment_field_unavailable", revision, field } as const;
  const selected = field === undefined
    ? options.preset && (!options.preset.collections || options.preset.collections.includes(collection))
      ? selectContentPreset(options.preset, collection, localeAtAudit ?? undefined) : undefined
    : { name: fragmentPreset!, source: "fragment_field" as const };
  if (!selected?.name) return { status: "not_checked", reason: "preset_not_configured", revision } as const;
  const contentFields = field === undefined ? options.contentFieldsByCollection?.[collection] : [field];
  // A revision stores field data, not the historical SEO panel or rendered HTML.
  const item: EmdashContentItem = { id, type: collection, slug: null, status: "draft", locale: localeAtAudit,
    data: snapshot.data };
  const adapted = adaptContentItem(item, { route: `/revision/${encodeURIComponent(id)}`, blockMappings: options.blockMappings,
    ...(contentFields === undefined ? {} : { contentFields }),
    ...(options.descriptionFieldsByCollection?.[collection] === undefined ? {} : { descriptionFields: options.descriptionFieldsByCollection[collection] }),
    ...(options.metadataFieldsByCollection?.[collection] === undefined ? {} : { metadataFields: options.metadataFieldsByCollection[collection] }) });
  // A runtime provider replaces static evidence in the ordinary evaluator.
  // It cannot safely be called for a historical revision without a revision-aware contract.
  const evidence = options.evidenceProviderConfigured ? undefined : options.preset?.evidenceSources;
  const report = await options.evaluate(adapted.document, selected.name, options.preset?.sources, evidence);
  const latest = await ctx.content.get(collection, id);
  const auditId = crypto.randomUUID();
  const result = { status: "evaluated" as const, audit_id: auditId, collection, id, revision,
    preset: selected.name, selection_source: selected.source, locale_at_audit: localeAtAudit, scope: field === undefined ? "content" as const : "fragment" as const, ...(field === undefined ? {} : { field }), limitation: LIMITATION,
    assurance: {
      purpose: "editorial_review" as const,
      publication_authorization: "not_granted" as const,
      rendered_page: "not_checked" as const,
      source_freshness: "not_checked" as const,
      runtime_providers: "not_invoked" as const,
      locale_source: "current_record" as const,
    },
    input_coverage: {
      body: {
        coverage: cmsBodyCoverage(adapted.document),
        capabilities: adapted.document.body === undefined ? [] :
          adapted.document.body_capabilities ?? ["text", "headings", "paragraphs", "links"],
        field_selection: contentFields === undefined ? "automatic" as const : "configured" as const,
        configured_fields: contentFields ?? null,
        unmapped_components: adapted.meta.unmappedComponents ?? { entries: [], truncated: false },
      },
      description: {
        field_selection: options.descriptionFieldsByCollection?.[collection] === undefined ? "automatic" as const : "configured" as const,
        configured_fields: options.descriptionFieldsByCollection?.[collection] ?? null,
        source_observed: adapted.document.description_source_observed === true,
        value_supplied: adapted.document.description !== undefined,
      },
      metadata: {
        configured_fields: Object.keys(options.metadataFieldsByCollection?.[collection] ?? {}),
        observed_fields: adapted.document.observed_metadata_fields ?? [],
        supplied_fields: Object.keys(adapted.document.meta ?? {}),
        unavailable_fields: adapted.document.unavailable_metadata_fields ?? [],
      },
      rendered_page: "not_observed" as const,
    },
    source_paths: { blocks: adapted.meta.bodySourcePaths ?? null, spans: adapted.meta.bodySourceSpanPaths ?? null }, report };
  await ctx.kv.set(reportKey(auditId), result);
  return { ...result, revision_status: revisionStatus(latest, revision), locale_status: localeStatus(result.locale_at_audit, latest) };
}

function revisionStatus(current: Current | null, revision: string): "draft" | "published" | "stale" | "unknown" {
  if (!current) return "unknown";
  if (current.draftRevisionId === revision) return "draft";
  if (current.liveRevisionId === revision) return "published";
  // A missing or malformed pointer may still identify this revision. Only
  // conclude it is stale when both pointers were observed and are valid.
  const observed = (value: unknown) => value === null || (typeof value === "string" && value.trim().length > 0);
  return observed(current.draftRevisionId) && observed(current.liveRevisionId) ? "stale" : "unknown";
}

function observedLocale(locale: unknown): string | null {
  return typeof locale === "string" && locale.trim().length > 0 ? locale : null;
}

function localeStatus(audited: unknown, current: Current | null): "unchanged" | "changed" | "unknown" {
  const prior = observedLocale(audited);
  const now = observedLocale(current?.locale);
  return prior === null || now === null ? "unknown" : prior === now ? "unchanged" : "changed";
}

/** Re-check status at read time; a stored report does not prove currentness. */
export async function getRevisionAudit(ctx: RevisionContext, auditId: string) {
  const stored = await ctx.kv.get<{ collection: string; id: string; revision: string; locale_at_audit?: string | null }>(reportKey(auditId));
  if (!stored) return { status: "not_checked", reason: "audit_unavailable" } as const;
  const current = ctx.content.get ? await ctx.content.get(stored.collection, stored.id) : null;
  return { ...stored, revision_status: revisionStatus(current, stored.revision), locale_status: localeStatus(stored.locale_at_audit, current) };
}
