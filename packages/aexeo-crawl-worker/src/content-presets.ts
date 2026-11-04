/** Optional content evaluation for the sandbox sidecar. */
export interface ContentPresetEnv {
  CONTENT_PRESET?: string;
  CONTENT_PRESET_BY_COLLECTION_JSON?: string;
  CONTENT_PRESET_SOURCES_JSON?: string;
}

function stringMap(raw: string | undefined, name: string): Record<string, string> {
  if (!raw) return {};
  const value: unknown = JSON.parse(raw);
  if (typeof value !== "object" || value === null || Array.isArray(value) || Object.values(value).some((item) => typeof item !== "string")) {
    throw new Error(`${name} must be a JSON object with string values`);
  }
  return value as Record<string, string>;
}

export function contentPresetFindings(
  documents: unknown[],
  collections: unknown,
  env: ContentPresetEnv,
  evaluate: (documentJson: string, name: string, sourcesJson?: string | null) => string,
): unknown[] {
  const byCollection = stringMap(env.CONTENT_PRESET_BY_COLLECTION_JSON, "CONTENT_PRESET_BY_COLLECTION_JSON");
  const sources = stringMap(env.CONTENT_PRESET_SOURCES_JSON, "CONTENT_PRESET_SOURCES_JSON");
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
    const report = JSON.parse(evaluate(JSON.stringify(document), name, JSON.stringify(sources))) as {
      gates: Array<{ gate_id: string; status: string; reason: string | null; findings: unknown[] }>;
    };
    for (const gate of report.gates) {
      if (gate.status === "not_checked") throw new Error(`content preset incomplete: ${gate.gate_id}: ${gate.reason}`);
      findings.push(...gate.findings);
    }
  }
  return findings;
}
