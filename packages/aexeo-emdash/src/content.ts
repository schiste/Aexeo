import { addProseVarianceMeasurements, emitTelemetry, listReviewMeasurements } from "./telemetry.js";
import type { ListReviewTelemetry } from "./telemetry.js";
import { evaluateContent as evaluateContentWasm, evaluateContentCollection as evaluateCollectionWasm, projectContentInput as projectContentInputWasm, projectContentAdapter as projectContentAdapterWasm, projectLinkDestinations as projectLinkDestinationsWasm } from "./wasm-init.js";
import type { EmdashDocument, Finding } from "./types.js";
import type { AdaptedContent } from "./adapter.js";

/** Adapt an EmDash row using explicit host field and component mappings. */
export { adaptContentItem } from "./adapter.js";
export type { AdaptContentOptions, AdaptedContent, EmdashContentItem, CustomBlockMappings, DescriptionField, MetadataFields } from "./adapter.js";

export interface ContentGateResult {
  gate_id: string;
  status: "passed" | "failed" | "not_checked";
  blocking: boolean;
  scope: "document" | "collection" | "site";
  required_capability: "text" | "source_text" | "metadata" | "headings" | "heading_levels" | "structured_lists" | "classified_paragraphs" | "paragraphs" | "links" | "link_destinations" | "link_reachability" | "literal_markdown_links" | "collection_metadata" | "site" | "extension" | "text_and_evidence";
  findings: Finding[];
  reason: string | null;
  measurements?: Record<string, number>;
}

/** Coordinates inside the exact Portable Text snapshot sent for evaluation. */
export interface EmdashSourceLocation {
  gate_id: string;
  finding_index: number;
  block_index: number;
  span_index: number;
  /** Zero-based UTF-16 code unit offset in the span text. */
  utf16_offset: number;
}

export interface ContentReport {
  preset: string;
  input_id: string;
  gates: ContentGateResult[];
  summary?: { passed: number; failed: number; not_checked: number; blocking_failed: number; blocking_not_checked: number };
  applied_fixes: string[];
  /** Omitted for collection reports; link findings point to the first marked span. */
  source_locations?: EmdashSourceLocation[];
  /** Emitted positional findings; report limits can omit additional findings. */
  source_mapping?: { eligible: number; mapped: number; unmapped: number };
}

/** Local, opt-in aggregate metrics; contains no document text or identifiers. */
export interface ContentEvaluationTelemetry {
  /** End-to-end wrapper duration, including serialization and WASM initialization. */
  evaluation_ms: number;
  source_locations_mapped: number;
  /** Omitted with older WASM bridges. */
  source_locations_eligible?: number;
  source_locations_unmapped?: number;
  /** Aggregate destination occurrences; omitted when the gate did not measure them. */
  link_destinations?: { checked: number; invalid: number };
  link_reachability?: { checked: number; not_found: number; unchecked: number; not_applicable?: number };
  /** Aggregate counts, omitted when no prose-variance gate was measured. */
  prose_variance?: { measured_gates: number; list_paragraphs_excluded: number; windows_checked: number };
  /** List observations are summed per gate; measured counts are absent if no gate measured them. */
  list_review?: ListReviewTelemetry;
}

/** Exact projected input bound to external extension results; may contain private content. */
export interface ExtensionInput {
  id: string;
  text: string | null;
  source_text: string | null;
  headings: string[] | null;
  metadata: Record<string, string> | null;
  evidence_sources: Record<string, string> | null;
  /** Metadata source coverage; retain this unchanged in the result input. */
  metadata_coverage?: {
    observed_fields: string[] | null;
    unavailable_fields: string[];
    description_source_observed: boolean | null;
  };
}

export interface ExternalGateResult {
  extension: string;
  parameters: Record<string, string>;
  input: ExtensionInput;
  diagnostics: Array<{ message: string; line: number; column: number }>;
}

/** Project the same input the evaluator will give a trusted extension. */
export async function projectContentInput(document: EmdashDocument, evidenceSources?: Record<string, string>): Promise<ExtensionInput> {
  return JSON.parse(await projectContentInputWasm(JSON.stringify(document), evidenceSources === undefined ? undefined : JSON.stringify(evidenceSources))) as ExtensionInput;
}

/** Body observations for the CLI's adapter-json format; contains content and URLs. */
export interface ContentAdapterProjection {
  schema: "aexeo.content-adapter/v1";
  text: string | null;
  literal_text?: string;
  /** Path coordinates are relative to the snapshot documented by the projection function. */
  native_source_map?: { spans: Array<{ start: number; end: number; path: Array<string | number>; utf16_start?: number }>; heading_paths?: Array<Array<string | number> | null>; link_paths?: Array<Array<string | number> | null>; list_paths?: Array<Array<string | number> | null> };
  metadata?: Record<string, string>;
  observed_metadata_fields?: string[];
  headings?: string[];
  heading_levels?: number[];
  paragraphs?: Array<{ start: number; end: number; list_item?: boolean }>;
  block_ends?: number[];
  lists?: Array<{ item_count: number; depth: number }>;
  link_destinations?: string[];
}

/** Aggregate projection coverage; never includes content, URLs or document identity. */
export interface ContentAdapterProjectionTelemetry {
  projection_ms: number;
  text_observed: boolean;
  headings_observed: boolean;
  paragraphs_observed: boolean;
  lists_observed: boolean;
  links_observed: boolean;
  literal_projection_supplied: boolean;
  metadata_values?: number;
  observed_metadata_fields?: number;
  native_source_spans?: number;
  native_structure_paths?: { headings?: number; links?: number; lists?: number };
}

/** Export observed body and CMS metadata; native paths identify the projected document. */
export async function projectContentAdapter(
  document: EmdashDocument,
  onTelemetry?: (event: ContentAdapterProjectionTelemetry) => void,
): Promise<ContentAdapterProjection> {
  const started = onTelemetry ? performance.now() : 0;
  const projection = JSON.parse(await projectContentAdapterWasm(JSON.stringify(document))) as ContentAdapterProjection;
  if (onTelemetry) reportProjectionTelemetry(projection, started, onTelemetry);
  return projection;
}

function reportProjectionTelemetry(projection: ContentAdapterProjection, started: number, onTelemetry: (event: ContentAdapterProjectionTelemetry) => void): void {
  emitTelemetry(onTelemetry, {
    projection_ms: performance.now() - started,
    text_observed: projection.text !== null,
    headings_observed: projection.headings !== undefined,
    paragraphs_observed: projection.paragraphs !== undefined,
    lists_observed: projection.lists !== undefined,
    links_observed: projection.link_destinations !== undefined,
    literal_projection_supplied: projection.literal_text !== undefined,
    ...(projection.metadata === undefined ? {} : { metadata_values: Object.keys(projection.metadata).length }),
    ...(projection.observed_metadata_fields === undefined ? {} : { observed_metadata_fields: projection.observed_metadata_fields.length }),
    ...(projection.native_source_map === undefined ? {} : {
      native_source_spans: projection.native_source_map.spans.length,
      native_structure_paths: {
        ...(projection.native_source_map.heading_paths === undefined ? {} : { headings: projection.native_source_map.heading_paths.filter(Boolean).length }),
        ...(projection.native_source_map.link_paths === undefined ? {} : { links: projection.native_source_map.link_paths.filter(Boolean).length }),
        ...(projection.native_source_map.list_paths === undefined ? {} : { lists: projection.native_source_map.list_paths.filter(Boolean).length }),
      },
    }),
  });
}

/** Project an unchanged adapted CMS row; native paths are relative to the complete row. */
export async function projectAdaptedContentAdapter(
  adapted: AdaptedContent,
  onTelemetry?: (event: ContentAdapterProjectionTelemetry) => void,
): Promise<ContentAdapterProjection> {
  const started = onTelemetry ? performance.now() : 0;
  const projection = await projectContentAdapter(adapted.document);
  if (projection.native_source_map) {
    const encoder = new TextEncoder();
    const mapPath = (original: Array<string | number>): Array<string | number> | null => {
      const [body, block, children, child, text] = original;
      if (original.length !== 5 || body !== "body" || children !== "children" || text !== "text" ||
          typeof block !== "number" || typeof child !== "number" || !Number.isSafeInteger(block) || !Number.isSafeInteger(child) || block < 0 || child < 0) return null;
      const blockPath = adapted.meta.bodySourcePaths?.[block];
      const path = blockPath ? [...blockPath, "children", child, "text"] : adapted.meta.bodySourceSpanPaths?.[block]?.[child];
      const validPath = path && path.length > 0 && path.length <= 32 && path.every(part =>
        typeof part === "string" ? part.length > 0 && encoder.encode(part).length <= 256 : Number.isSafeInteger(part) && part >= 0,
      );
      return validPath ? path : null;
    };
    projection.native_source_map.spans = projection.native_source_map.spans.flatMap(span => {
      const path = mapPath(span.path);
      return path ? [{ ...span, path }] : [];
    });
    for (const key of ["heading_paths", "link_paths", "list_paths"] as const) {
      const paths = projection.native_source_map[key];
      if (paths) projection.native_source_map[key] = paths.map(path => path ? mapPath(path) : null);
    }
    // The CLI binds and reads the entire projection under this shared limit.
    // Paths are optional when their serialized overhead exceeds the budget.
    if (encoder.encode(JSON.stringify(projection)).length > 16 * 1024 * 1024) delete projection.native_source_map;
  }
  if (onTelemetry) reportProjectionTelemetry(projection, started, onTelemetry);
  return projection;
}

/** Snapshot and link occurrences in adapter order. This may contain private content and URLs. */
export interface LinkDestinationProjection {
  document: EmdashDocument;
  destinations: string[];
}

export interface BoundLinkObservations extends LinkDestinationProjection {
  observations: Array<"reachable" | "not_found" | "not_checked" | "not_applicable">;
}

/** Return null when link coverage is unavailable. The returned snapshot must not be modified. */
export async function projectLinkDestinations(document: EmdashDocument): Promise<LinkDestinationProjection | null> {
  return JSON.parse(await projectLinkDestinationsWasm(JSON.stringify(document))) as LinkDestinationProjection | null;
}

/** Evaluate a document locally using the built-in or supplied named TOML presets. */
export async function evaluateContent(
  document: EmdashDocument,
  preset: string,
  sources: Record<string, string> = {},
  evidenceSources?: Record<string, string>,
  onTelemetry?: (event: ContentEvaluationTelemetry) => void,
  externalResults?: ExternalGateResult[],
  linkObservations?: BoundLinkObservations,
): Promise<ContentReport> {
  const started = onTelemetry ? performance.now() : 0;
  const documentJson = JSON.stringify(document);
  const sourcesJson = JSON.stringify(sources);
  const result = linkObservations !== undefined
    ? await evaluateContentWasm(documentJson, preset, sourcesJson, evidenceSources === undefined ? undefined : JSON.stringify(evidenceSources), externalResults === undefined ? undefined : JSON.stringify(externalResults), JSON.stringify(linkObservations))
    : externalResults !== undefined
    ? await evaluateContentWasm(documentJson, preset, sourcesJson, evidenceSources === undefined ? undefined : JSON.stringify(evidenceSources), JSON.stringify(externalResults))
    : evidenceSources === undefined
      ? await evaluateContentWasm(documentJson, preset, sourcesJson)
      : await evaluateContentWasm(documentJson, preset, sourcesJson, JSON.stringify(evidenceSources));
  const report = JSON.parse(result) as ContentReport;
  if (onTelemetry) {
    const event: ContentEvaluationTelemetry = { evaluation_ms: performance.now() - started, source_locations_mapped: report.source_locations?.length ?? 0 };
    if (report.source_mapping) {
      event.source_locations_eligible = report.source_mapping.eligible;
      event.source_locations_unmapped = report.source_mapping.unmapped;
    }
    const listReview = listReviewMeasurements(report.gates);
    if (listReview !== undefined) event.list_review = listReview;
    const proseVariance = addProseVarianceMeasurements(report.gates);
    if (proseVariance !== undefined) event.prose_variance = proseVariance;
    let notApplicableTotal = 0;
    let notApplicableComplete = true;
    for (const gate of report.gates) {
      if (gate.required_capability === "link_reachability") {
        const measurements = gate.measurements;
        const count = measurements?.not_applicable_destinations;
        if (count === undefined || measurements?.checked_destinations === undefined ||
            measurements.not_found_destinations === undefined || measurements.unchecked_destinations === undefined) {
          notApplicableComplete = false;
        } else {
          notApplicableTotal += count;
        }
      }
      if (gate.required_capability === "link_reachability" && gate.measurements) {
        const { checked_destinations, not_found_destinations, unchecked_destinations } = gate.measurements;
        if (checked_destinations !== undefined && not_found_destinations !== undefined && unchecked_destinations !== undefined) {
          event.link_reachability ??= { checked: 0, not_found: 0, unchecked: 0 };
          event.link_reachability.checked += checked_destinations;
          event.link_reachability.not_found += not_found_destinations;
          event.link_reachability.unchecked += unchecked_destinations;
        }
      }
      const checked = gate.measurements?.checked_destinations;
      const invalid = gate.measurements?.invalid_destinations;
      if (checked !== undefined && invalid !== undefined) {
        event.link_destinations ??= { checked: 0, invalid: 0 };
        event.link_destinations.checked += checked;
        event.link_destinations.invalid += invalid;
      }
    }
    if (event.link_reachability && notApplicableComplete) {
      event.link_reachability.not_applicable = notApplicableTotal;
    }
    emitTelemetry(onTelemetry, event);
  }
  return report;
}

/** Mark known incomplete collections explicitly; omitted options assume completeness. */
export async function evaluateContentCollection(
  documents: EmdashDocument[],
  preset: string,
  sources: Record<string, string> = {},
  options: { complete?: boolean } = {},
): Promise<ContentReport> {
  const input = options.complete === false ? { documents, complete: false } : documents;
  return JSON.parse(await evaluateCollectionWasm(JSON.stringify(input), preset, JSON.stringify(sources))) as ContentReport;
}
