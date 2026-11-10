/* tslint:disable */
/* eslint-disable */

/**
 * Evaluate one EmDash document using named TOML preset sources. The source map
 * is JSON: {"preset-name": "TOML text"}; built-ins are available by default.
 */
export function evaluateContent(document_json: string, preset_name: string, sources_json?: string | null, evidence_json?: string | null, extensions_json?: string | null, link_observations_json?: string | null): string;

/**
 * Evaluate collection-scoped gates on a complete set of EmDash documents.
 */
export function evaluateContentCollection(documents_json: string, preset_name: string, sources_json?: string | null): string;

export function evaluateDocuments(documents_json: string, config_json?: string | null): string;

/**
 * Generate the LLM authoring prompt for a truth manifest. The plugin calls
 * this when the editor opens "Generate authoring prompt" — the returned
 * string is what the editor pastes into Claude/GPT/etc.
 */
export function generateFactsPrompt(documents_json: string): string;

/**
 * Return the exact projected input that external extensions must bind to.
 */
export function projectContentInput(document_json: string, evidence_json?: string | null): string;

/**
 * Project the exact snapshot and ordered link occurrences for an external observer.
 * Returns JSON null when the adapter cannot supply complete link destinations.
 */
export function projectLinkDestinations(document_json: string): string;

export function scoreIntelligence(documents_json: string, manifest_json?: string | null): string;

/**
 * Validate a candidate `facts.json`. Returns a JSON object with two top-level
 * fields: `validation` (shape / schema check) and `assessment` (truth-layer
 * audit including mismatches against the site). The plugin renders this in
 * the validate-paste UI.
 */
export function validateFactsManifest(manifest_json: string, documents_json: string): string;
