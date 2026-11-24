import { adaptContentItem, type CustomBlockMappings, type DescriptionField, type MetadataFields, type EmdashContentItem } from "./adapter.js";
import { validLinkObservations } from "./content-evaluator.js";
import type { ContentPresetOptions, LinkProviderTelemetry } from "./content-evaluator.js";
import { emitTelemetry } from "./telemetry.js";
import { selectFragmentPreset } from "./fragment-selection.js";
import { collectionListStructureAttested, fragmentListStructureAttested } from "./list-structure-attestation.js";
import { selectContentPreset } from "./preset-selection.js";
import type { ContentReport, ContentEvaluationTelemetry, ExternalGateResult, BoundLinkObservations, LinkDestinationProjection } from "./content.js";
import type { SandboxCtx } from "./plugin.js";
import type { EmdashDocument } from "./types.js";
import { cmsBodyCoverage } from "./cms-coverage.js";

export interface RevisionAuditRequest { collection: string; id: string; revision: string; field?: string }
export interface CandidateAuditRequest extends RevisionAuditRequest { field: string; candidate: unknown }
const MAX_CANDIDATE_BYTES = 1024 * 1024;
interface Revision { id: string; collection: string; entryId: string; data: Record<string, unknown> }
type Current = EmdashContentItem & { draftRevisionId?: string | null; liveRevisionId?: string | null };
type RevisionContext = SandboxCtx & { content: SandboxCtx["content"] & {
  get?: (collection: string, id: string) => Promise<Current | null>;
  getRevision?: (collection: string, id: string, revision: string) => Promise<Revision | null>;
} };
/** Explicitly installed trusted observer for one immutable host-provided revision.
 * Destinations and revision identifiers may be sensitive; hosts control network policy,
 * access, timeout and freshness. The provider must not substitute another revision. */
export type RevisionLinkObservationProvider = (request: {
  collection: string; id: string; revision: string; field?: string;
  locale: string | undefined; preset: string; destinations: readonly string[];
}) => Promise<BoundLinkObservations["observations"]>;

/** Trusted host projection for the locale of the exact requested revision.
 * The host must establish that the value belongs to that revision. */
export type RevisionLocaleProvider = (request: {
  collection: string; id: string; revision: string;
}) => Promise<string | null>;

/** Aggregate-only observation of the locale provider; no content or identifiers. */
export interface RevisionLocaleTelemetry {
  duration_ms: number;
  status: "success" | "unavailable" | "error";
}

export interface RevisionAuditOptions {
  preset?: ContentPresetOptions | undefined;
  blockMappings?: CustomBlockMappings | undefined;
  contentFieldsByCollection?: Record<string, readonly string[]> | undefined;
  descriptionFieldsByCollection?: Record<string, readonly DescriptionField[]> | undefined;
  titleFieldsByCollection?: Record<string, readonly DescriptionField[]> | undefined;
  metadataFieldsByCollection?: Record<string, MetadataFields> | undefined;
  evidenceProviderConfigured?: boolean;
  evaluate: (document: EmdashDocument, preset: string, sources?: Record<string, string>, evidence?: Record<string, string>, onTelemetry?: (event: ContentEvaluationTelemetry) => void, externalResults?: ExternalGateResult[], linkObservations?: BoundLinkObservations) => Promise<ContentReport>;
  projectLinks?: (document: EmdashDocument) => Promise<LinkDestinationProjection | null>;
  revisionLinkObservations?: RevisionLinkObservationProvider | undefined;
  revisionLocale?: RevisionLocaleProvider | undefined;
  onRevisionLocaleTelemetry?: ((event: RevisionLocaleTelemetry) => void) | undefined;
  onLinkProviderTelemetry?: ((event: LinkProviderTelemetry) => void) | undefined;
  onContentEvaluationTelemetry?: ((event: ContentEvaluationTelemetry) => void) | undefined;
}

const reportKey = (id: string) => `revision-audit:${id}`;

/** Audit a proposed fragment against a historical revision without writing content. */
export async function auditCandidate(ctx: RevisionContext, input: CandidateAuditRequest, options: RevisionAuditOptions) {
  if (typeof input.field !== "string" || input.field.trim().length === 0) return { status: "not_checked", reason: "candidate_field_invalid" } as const;
  let encoded: Uint8Array<ArrayBuffer>;
  let value: unknown;
  try {
    const serialized = JSON.stringify(input.candidate);
    if (serialized === undefined) return { status: "not_checked", reason: "candidate_invalid" } as const;
    encoded = new TextEncoder().encode(serialized);
    value = JSON.parse(serialized) as unknown;
  } catch {
    return { status: "not_checked", reason: "candidate_invalid" } as const;
  }
  if (encoded.byteLength > MAX_CANDIDATE_BYTES) return { status: "not_checked", reason: "candidate_too_large" } as const;
  const hash = await crypto.subtle.digest("SHA-256", encoded);
  const digest = Array.from(new Uint8Array(hash), (byte) => byte.toString(16).padStart(2, "0")).join("");
  return auditRevisionInternal(ctx, input, options, { value, digest });
}

/** Audit the exact host-provided historical data; never read or write article content. */
export async function auditRevision(ctx: RevisionContext, input: RevisionAuditRequest, options: RevisionAuditOptions) {
  return auditRevisionInternal(ctx, input, options);
}

async function auditRevisionInternal(ctx: RevisionContext, input: RevisionAuditRequest, options: RevisionAuditOptions, candidate?: { value: unknown; digest: string }) {
  const { collection, id, revision } = input;
  if (!ctx.content.get || !ctx.content.getRevision) return { status: "not_checked", reason: "revision_access_unavailable" } as const;
  let current: Current | null;
  let snapshot: Revision | null;
  try {
    [current, snapshot] = await Promise.all([ctx.content.get(collection, id), ctx.content.getRevision(collection, id, revision)]);
  } catch {
    // Host errors may contain private data; return only a bounded diagnostic.
    return { status: "not_checked", reason: "revision_read_failed" } as const;
  }
  if (!current || !snapshot || snapshot.id !== revision || snapshot.collection !== collection || snapshot.entryId !== id) {
    return { status: "not_checked", reason: "revision_unavailable" } as const;
  }
  if (!matchesCurrentIdentity(current, collection, id)) {
    return { status: "not_checked", reason: "current_identity_mismatch" } as const;
  }
  // Host data is an external runtime boundary: malformed historical payloads
  // are unavailable, never an empty document or an exception to the caller.
  if (snapshot.data === null || typeof snapshot.data !== "object" || Array.isArray(snapshot.data)) {
    return { status: "not_checked", reason: "revision_data_invalid", revision } as const;
  }
  let localeAtAudit = observedLocale(current.locale);
  if (options.revisionLocale) {
    const started = options.onRevisionLocaleTelemetry ? performance.now() : 0;
    let status: RevisionLocaleTelemetry["status"] = "error";
    try {
      const locale = observedLocale(await options.revisionLocale({ collection, id, revision }));
      if (locale === null || locale.length > 128 || locale.trim() !== locale || !/^[A-Za-z0-9]+(?:[-_][A-Za-z0-9]+)*$/.test(locale)) {
        status = "unavailable";
        return { status: "not_checked", reason: "revision_locale_unavailable", revision } as const;
      }
      localeAtAudit = locale;
      status = "success";
    } catch {
      return { status: "not_checked", reason: "revision_locale_read_failed", revision } as const;
    } finally {
      if (options.onRevisionLocaleTelemetry) emitTelemetry(options.onRevisionLocaleTelemetry, { duration_ms: performance.now() - started, status });
    }
  }
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
    data: candidate === undefined ? snapshot.data : { ...snapshot.data, [field!]: candidate.value } };
  const adapted = adaptContentItem(item, { route: `/revision/${encodeURIComponent(id)}`, blockMappings: options.blockMappings,
    ...(contentFields === undefined ? {} : { contentFields }),
    ...(options.descriptionFieldsByCollection?.[collection] === undefined ? {} : { descriptionFields: options.descriptionFieldsByCollection[collection] }),
    ...(options.titleFieldsByCollection?.[collection] === undefined ? {} : { titleFields: options.titleFieldsByCollection[collection] }),
    ...(options.metadataFieldsByCollection?.[collection] === undefined ? {} : { metadataFields: options.metadataFieldsByCollection[collection] }) });
  const listStructureAttested = field === undefined
    ? collectionListStructureAttested(options.preset?.revisionListStructureCompleteByCollection, collection)
    : fragmentListStructureAttested(options.preset?.revisionFragmentListStructureCompleteByCollection, collection, field);
  if (listStructureAttested && candidate === undefined) adapted.document.list_structure_complete = true;
  // A runtime provider replaces static evidence in the ordinary evaluator.
  // It cannot safely be called for a historical revision without a revision-aware contract.
  const evidence = options.evidenceProviderConfigured ? undefined : options.preset?.evidenceSources;
  let report = options.onContentEvaluationTelemetry
    ? await options.evaluate(adapted.document, selected.name, options.preset?.sources, evidence, options.onContentEvaluationTelemetry)
    : await options.evaluate(adapted.document, selected.name, options.preset?.sources, evidence);
  let revisionObserverInvoked = false;
  if (candidate === undefined && options.revisionLinkObservations && report.gates.some((gate) => gate.required_capability === "link_reachability")) {
    const started = options.onLinkProviderTelemetry ? performance.now() : 0;
    let status: LinkProviderTelemetry["status"] = "error";
    let reason: string | undefined;
    try {
      const projection = options.projectLinks ? await options.projectLinks(adapted.document) : null;
      if (projection === null) {
        status = "unavailable";
        reason = "link projection unavailable from adapter";
      } else {
        revisionObserverInvoked = true;
        const observations = await options.revisionLinkObservations({ collection, id, revision,
          ...(field === undefined ? {} : { field }), locale: localeAtAudit ?? undefined,
          preset: selected.name, destinations: [...projection.destinations] });
        if (!validLinkObservations(observations, projection.destinations.length)) throw new Error("invalid link observations");
        report = await options.evaluate(adapted.document, selected.name, options.preset?.sources, evidence,
          options.onContentEvaluationTelemetry, undefined, { ...projection, observations });
        status = "success";
      }
    } catch {
      // Keep local results and do not disclose private provider error details.
      reason = "link observation failed";
    } finally {
      if (options.onLinkProviderTelemetry) emitTelemetry(options.onLinkProviderTelemetry, { duration_ms: performance.now() - started, status });
    }
    if (reason) report = { ...report, gates: report.gates.map((gate) => gate.required_capability === "link_reachability" && gate.status === "not_checked" ? { ...gate, reason } : gate) };
  }
  const latest = await currentOrUnknown(ctx, collection, id);
  const auditId = crypto.randomUUID();
  const localeProviderInvoked = options.revisionLocale !== undefined;
  const runtimeProviders = localeProviderInvoked
    ? revisionObserverInvoked ? "revision_locale_and_link_observer_invoked" as const : "revision_locale_provider_invoked" as const
    : revisionObserverInvoked ? "revision_link_observer_invoked" as const : "not_invoked" as const;
  const providerLimitation = {
    not_invoked: "Only local configured content checks run; runtime providers are not invoked.",
    revision_link_observer_invoked: "An explicitly configured revision link observer ran; other runtime providers were not invoked.",
    revision_locale_provider_invoked: "An explicitly configured revision locale provider ran; other runtime providers were not invoked.",
    revision_locale_and_link_observer_invoked: "Explicitly configured revision locale and link providers ran; other runtime providers were not invoked.",
  }[runtimeProviders];
  const localeLimitation = localeProviderInvoked ? "the preset locale is supplied by the host for the requested revision" : "the preset locale is taken from the current record";
  const limitation = `${candidate === undefined ? "" : "This is an unsaved proposed fragment, not the contents of the base revision; revision-specific link observations and list-structure attestations were not used. "}${providerLimitation} Rendered page metadata and source freshness are not verified; ${localeLimitation}; passing does not establish factual accuracy or currentness and does not authorize publication.`;
  const result = { status: "evaluated" as const, audit_id: auditId, collection, id, revision,
    ...(candidate === undefined ? {} : { candidate: { status: "unsaved" as const, field: field!, sha256_json: candidate.digest } }),
    preset: selected.name, selection_source: selected.source, locale_at_audit: localeAtAudit, scope: field === undefined ? "content" as const : "fragment" as const, ...(field === undefined ? {} : { field }), limitation,
    assurance: {
      purpose: "editorial_review" as const,
      publication_authorization: "not_granted" as const,
      rendered_page: "not_checked" as const,
      source_freshness: "not_checked" as const,
      runtime_providers: runtimeProviders,
      locale_source: localeProviderInvoked ? "revision_provider" as const : "current_record" as const,
    },
    input_coverage: {
      body: {
        coverage: cmsBodyCoverage(adapted.document),
        projected_blocks: adapted.document.body?.length ?? null,
        capabilities: adapted.document.body === undefined ? [] :
          adapted.document.body_capabilities ?? ["text", "headings", "paragraphs", "links"],
        field_selection: contentFields === undefined ? "automatic" as const : "configured" as const,
        configured_fields: contentFields ?? null,
        unmapped_components: adapted.meta.unmappedComponents ?? { entries: [], truncated: false },
      },
      title: {
        field_selection: options.titleFieldsByCollection?.[collection] === undefined ? "automatic" as const : "configured" as const,
        configured_fields: options.titleFieldsByCollection?.[collection] ?? null,
        source_observed: adapted.document.observed_metadata_fields?.includes("title") === true,
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

function matchesCurrentIdentity(value: unknown, collection: string, id: string): value is Current {
  if (value === null || typeof value !== "object" || Array.isArray(value)) return false;
  const record = value as { id?: unknown; type?: unknown };
  return record.id === id && record.type === collection;
}

async function currentOrUnknown(ctx: RevisionContext, collection: string, id: string): Promise<Current | null> {
  try {
    const value = ctx.content.get ? await ctx.content.get(collection, id) : null;
    return matchesCurrentIdentity(value, collection, id) ? value : null;
  } catch {
    return null;
  }
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
  const current = await currentOrUnknown(ctx, stored.collection, stored.id);
  return { ...stored, revision_status: revisionStatus(current, stored.revision), locale_status: localeStatus(stored.locale_at_audit, current) };
}
