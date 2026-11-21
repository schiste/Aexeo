import { addProseVarianceMeasurements } from "../../aexeo-emdash/src/telemetry.js";
import { emptyPresetSelectionCounts, selectContentPreset } from "../../aexeo-emdash/src/preset-selection.js";
/** Optional content evaluation for the sandbox sidecar. */
export interface ContentPresetEnv {
  CONTENT_PRESET?: string;
  CONTENT_PRESET_BY_COLLECTION_JSON?: string;
  CONTENT_PRESET_BY_LOCALE_JSON?: string;
  CONTENT_PRESET_BY_COLLECTION_LOCALE_JSON?: string;
  CONTENT_PRESET_SOURCES_JSON?: string;
  CONTENT_EVIDENCE_SOURCES_JSON?: string;
  /** Optional Cloudflare service binding, configured by the operator. */
  CONTENT_GATE_PROVIDER?: { fetch(input: string, init: RequestInit): Promise<Response> };
  CONTENT_GATE_TELEMETRY?: string;
  CONTENT_PRESET_SELECTION_TELEMETRY?: string;
  CONTENT_PROSE_VARIANCE_TELEMETRY?: string;
}

const MAX_PROVIDER_BYTES = 2 * 1024 * 1024;
const PROVIDER_TIMEOUT_MS = 5000;

async function providerResults(provider: NonNullable<ContentPresetEnv["CONTENT_GATE_PROVIDER"]>, input: unknown, preset: string, collection: string | undefined): Promise<string> {
  const response = await provider.fetch("https://content-gate.internal/evaluate", {
    method: "POST", headers: { "content-type": "application/json" },
    body: JSON.stringify({ input, preset, collection: collection ?? null }),
    signal: AbortSignal.timeout(PROVIDER_TIMEOUT_MS),
  });
  if (!response.ok) throw new Error(`content gate provider returned HTTP ${response.status}`);
  if (Number(response.headers.get("content-length") ?? 0) > MAX_PROVIDER_BYTES) throw new Error("content gate provider response too large");
  if (!response.body) throw new Error("content gate provider returned an empty response");
  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let size = 0;
  let text = "";
  for (;;) {
    const { value, done } = await reader.read();
    if (done) break;
    size += value.byteLength;
    if (size > MAX_PROVIDER_BYTES) {
      await reader.cancel();
      throw new Error("content gate provider response too large");
    }
    text += decoder.decode(value, { stream: true });
  }
  text += decoder.decode();
  let parsed: unknown;
  try { parsed = JSON.parse(text); } catch { throw new Error("content gate provider returned invalid JSON"); }
  if (!Array.isArray(parsed)) throw new Error("content gate provider must return an array");
  return text;
}

function stringMap(raw: string | undefined, name: string): Record<string, string> {
  if (!raw) return {};
  const value: unknown = JSON.parse(raw);
  if (typeof value !== "object" || value === null || Array.isArray(value) || Object.values(value).some((item) => typeof item !== "string")) {
    throw new Error(`${name} must be a JSON object with string values`);
  }
  return value as Record<string, string>;
}

export async function contentPresetFindings(
  documents: unknown[],
  collections: unknown,
  env: ContentPresetEnv,
  evaluate: (documentJson: string, name: string, sourcesJson?: string | null, evidenceJson?: string | null, extensionsJson?: string | null) => string,
  project?: (documentJson: string, evidenceJson?: string | null) => string,
  assessments?: Record<string, unknown>,
): Promise<unknown[]> {
  const byCollection = stringMap(env.CONTENT_PRESET_BY_COLLECTION_JSON, "CONTENT_PRESET_BY_COLLECTION_JSON");
  const byLocale = stringMap(env.CONTENT_PRESET_BY_LOCALE_JSON, "CONTENT_PRESET_BY_LOCALE_JSON");
  const nestedRaw: unknown = env.CONTENT_PRESET_BY_COLLECTION_LOCALE_JSON ? JSON.parse(env.CONTENT_PRESET_BY_COLLECTION_LOCALE_JSON) : {};
  if (typeof nestedRaw !== "object" || nestedRaw === null || Array.isArray(nestedRaw) ||
      Object.values(nestedRaw).some((locales) => typeof locales !== "object" || locales === null || Array.isArray(locales) || Object.values(locales).some((value) => typeof value !== "string"))) {
    throw new Error("CONTENT_PRESET_BY_COLLECTION_LOCALE_JSON must be an object of locale maps with string values");
  }
  const byCollectionLocale = nestedRaw as Record<string, Record<string, string>>;
  const sources = stringMap(env.CONTENT_PRESET_SOURCES_JSON, "CONTENT_PRESET_SOURCES_JSON");
  const evidence = env.CONTENT_EVIDENCE_SOURCES_JSON === undefined ? undefined : stringMap(env.CONTENT_EVIDENCE_SOURCES_JSON, "CONTENT_EVIDENCE_SOURCES_JSON");
  const needsCollections = Object.keys(byCollection).length > 0 || Object.keys(byCollectionLocale).length > 0;
  if (needsCollections && (!Array.isArray(collections) || collections.length !== documents.length || collections.some((item) => typeof item !== "string"))) {
    throw new Error("collection names are required for every document when collection presets are configured");
  }
  const findings: unknown[] = [];
  const selection = { ...(env.CONTENT_PRESET === undefined ? {} : { name: env.CONTENT_PRESET }), byCollection, byLocale, byCollectionLocale };
  const sourcesJson = JSON.stringify(sources);
  const evidenceJson = evidence === undefined ? undefined : JSON.stringify(evidence);
  const counts = env.CONTENT_PRESET_SELECTION_TELEMETRY === "true" ? emptyPresetSelectionCounts() : undefined;
  let status = "error";
  let proseVariance: ReturnType<typeof addProseVarianceMeasurements>;
  try {
    for (const [index, document] of documents.entries()) {
      const collection = Array.isArray(collections) ? collections[index] as string | undefined : undefined;
      const locale = typeof document === "object" && document !== null && "lang" in document && typeof document.lang === "string" ? document.lang : undefined;
      const selected = selectContentPreset(selection, collection, locale);
      if (counts) counts[selected.source]++;
      const name = selected.name;
      if (name === "" && selected.source !== "fallback") throw new Error("empty selected preset name");
      if (!name) continue;
      const documentJson = JSON.stringify(document);
      type Report = {
        gates: Array<{ gate_id: string; status: string; blocking: boolean; scope: string; required_capability: string; reason: string | null; measurements?: Record<string, number>; findings: unknown[] }>;
        source_mapping?: { eligible: number; mapped: number; unmapped: number };
        source_locations?: Array<{ gate_id: string; finding_index: number; block_index: number; span_index: number; utf16_offset: number }>;
      };
      let report = JSON.parse(evaluate(documentJson, name, sourcesJson, evidenceJson)) as Report;
      if (env.CONTENT_GATE_PROVIDER !== undefined && report.gates.some((gate) => gate.required_capability === "extension")) {
        const started = env.CONTENT_GATE_TELEMETRY === "true" ? performance.now() : 0;
        let status = "error";
        try {
          if (!project) throw new Error("content gate projection is unavailable");
          const extensionsJson = await providerResults(env.CONTENT_GATE_PROVIDER, JSON.parse(project(documentJson, evidenceJson)), name, collection);
          report = JSON.parse(evaluate(documentJson, name, sourcesJson, evidenceJson, extensionsJson)) as Report;
          status = "success";
        } catch (error) {
          // Legacy clients cannot represent unchecked gates, so retain their failure.
          if (assessments === undefined) throw error;
          report = { ...report, gates: report.gates.map((gate) => gate.required_capability === "extension" && gate.status === "not_checked"
            ? { ...gate, reason: "external gate provider failed" } : gate) };
        } finally {
          if (env.CONTENT_GATE_TELEMETRY === "true") {
            console.info(JSON.stringify({ event: "external_gate_provider", duration_ms: performance.now() - started, status }));
          }
        }
      }
      if (env.CONTENT_PROSE_VARIANCE_TELEMETRY === "true") {
        proseVariance = addProseVarianceMeasurements(report.gates, proseVariance);
      }
      if (assessments !== undefined) {
        if (typeof document !== "object" || document === null || !("route" in document) || typeof document.route !== "string") throw new Error("document route is required for assessments");
        assessments[document.route] = {
          status: "evaluated", preset: name,
          ...(report.source_mapping === undefined ? {} : { source_mapping: report.source_mapping }),
          gates: report.gates.map(({ gate_id, status, blocking, scope, required_capability, reason, measurements }) =>
            ({ gate_id, status, blocking, scope, required_capability, reason, ...(measurements === undefined ? {} : { measurements }) })),
        };
      }
      const locations = new Map((report.source_locations ?? []).map((location) => [`${location.gate_id}:${location.finding_index}`, location]));
      for (const gate of report.gates) {
        if (gate.status === "not_checked" && assessments === undefined) throw new Error(`content preset incomplete: ${gate.gate_id}: ${gate.reason}`);
        findings.push(...gate.findings.map((finding, index) => {
          if (assessments === undefined) return finding; // Keep the legacy response shape.
          if (typeof finding !== "object" || finding === null || Array.isArray(finding)) throw new Error("invalid content finding");
          const location = locations.get(`${gate.gate_id}:${index}`);
          return { ...finding, aexeo_origin: "content_preset",
            ...(location === undefined ? {} : { aexeo_source_location: { block_index: location.block_index, span_index: location.span_index, utf16_offset: location.utf16_offset } }) };
        }));
      }
    }
    status = "success";
    return findings;
  } finally {
    if (env.CONTENT_PROSE_VARIANCE_TELEMETRY === "true") {
      try { console.info(JSON.stringify({ event: "content_prose_variance", status, ...(proseVariance === undefined ? {} : { prose_variance: proseVariance }) })); } catch { /* best-effort telemetry */ }
    }
    if (counts) {
      try { console.info(JSON.stringify({ event: "content_preset_selection", status, counts })); } catch { /* best-effort telemetry */ }
    }
  }
}
