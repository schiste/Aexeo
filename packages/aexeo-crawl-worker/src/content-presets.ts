/** Optional content evaluation for the sandbox sidecar. */
export interface ContentPresetEnv {
  CONTENT_PRESET?: string;
  CONTENT_PRESET_BY_COLLECTION_JSON?: string;
  CONTENT_PRESET_SOURCES_JSON?: string;
  CONTENT_EVIDENCE_SOURCES_JSON?: string;
  /** Optional Cloudflare service binding, configured by the operator. */
  CONTENT_GATE_PROVIDER?: { fetch(input: string, init: RequestInit): Promise<Response> };
  CONTENT_GATE_TELEMETRY?: string;
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
  const sources = stringMap(env.CONTENT_PRESET_SOURCES_JSON, "CONTENT_PRESET_SOURCES_JSON");
  const evidence = env.CONTENT_EVIDENCE_SOURCES_JSON === undefined ? undefined : stringMap(env.CONTENT_EVIDENCE_SOURCES_JSON, "CONTENT_EVIDENCE_SOURCES_JSON");
  const needsCollections = Object.keys(byCollection).length > 0;
  if (needsCollections && (!Array.isArray(collections) || collections.length !== documents.length || collections.some((item) => typeof item !== "string"))) {
    throw new Error("collection names are required for every document when collection presets are configured");
  }
  const findings: unknown[] = [];
  for (const [index, document] of documents.entries()) {
    const collection = Array.isArray(collections) ? collections[index] as string | undefined : undefined;
    const selected = collection !== undefined && Object.hasOwn(byCollection, collection) ? byCollection[collection] : undefined;
    const name = selected ?? env.CONTENT_PRESET;
    if (selected === "") throw new Error(`empty preset name for collection ${collection}`);
    if (!name) continue;
    const documentJson = JSON.stringify(document);
    const evidenceJson = evidence === undefined ? undefined : JSON.stringify(evidence);
    type Report = {
      gates: Array<{ gate_id: string; status: string; blocking: boolean; scope: string; required_capability: string; reason: string | null; measurements?: Record<string, number>; findings: unknown[] }>;
      source_mapping?: { eligible: number; mapped: number; unmapped: number };
      source_locations?: Array<{ gate_id: string; finding_index: number; block_index: number; span_index: number; utf16_offset: number }>;
    };
    const sourcesJson = JSON.stringify(sources);
    let report = JSON.parse(evaluate(documentJson, name, sourcesJson, evidenceJson)) as Report;
    let extensionsJson: string | undefined;
    if (env.CONTENT_GATE_PROVIDER !== undefined && report.gates.some((gate) => gate.required_capability === "extension")) {
      if (!project) throw new Error("content gate projection is unavailable");
      const started = env.CONTENT_GATE_TELEMETRY === "true" ? performance.now() : 0;
      let status = "error";
      try {
        extensionsJson = await providerResults(env.CONTENT_GATE_PROVIDER, JSON.parse(project(documentJson, evidenceJson)), name, collection);
        status = "success";
      } finally {
        if (env.CONTENT_GATE_TELEMETRY === "true") {
          console.info(JSON.stringify({ event: "external_gate_provider", duration_ms: performance.now() - started, status }));
        }
      }
    }
    if (extensionsJson !== undefined) {
      report = JSON.parse(evaluate(documentJson, name, sourcesJson, evidenceJson, extensionsJson)) as Report;
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
  return findings;
}
